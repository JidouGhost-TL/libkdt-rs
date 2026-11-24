// SPDX-License-Identifier: MIT OR Apache-2.0

//! Clean-room synthetic integration tests for `libkdt` (no copyrighted bytes).

use libkdt::{
    detect_version, KdtCommand, KdtEvent, KdtPlayer, KdtSequence, KdtSequenceView, KdtVersion,
    ProgramAttr, RenderConfig, ReverbPreset, ToneAttr, VabBank, VabBankView,
};
use libspu2::registers::ps1;

// ---------------------------------------------------------------------------
// Synthetic asset builders (Ghidra-verified KDT grammar)
// ---------------------------------------------------------------------------

fn build_kdt_bytes(magic: [u8; 4], ppqn: u16, streams: &[&[u8]]) -> Vec<u8> {
    let version = KdtVersion::from_magic(magic).unwrap();
    let track_count = streams.len() as u16;
    let data_offset = version.data_offset(track_count);
    let total = data_offset + streams.iter().map(|s| s.len()).sum::<usize>();
    let mut out = vec![0u8; total];
    out[0..4].copy_from_slice(&magic);
    out[4..8].copy_from_slice(&(total as u32).to_le_bytes());
    out[8..10].copy_from_slice(&ppqn.to_le_bytes());
    out[12..14].copy_from_slice(&track_count.to_le_bytes());
    for (i, s) in streams.iter().enumerate() {
        out[16 + i * 2..18 + i * 2].copy_from_slice(&(s.len() as u16).to_le_bytes());
    }
    let mut cur = data_offset;
    for s in streams {
        out[cur..cur + s.len()].copy_from_slice(s);
        cur += s.len();
    }
    out
}

fn synthetic_adpcm() -> Vec<u8> {
    let mut block = [0u8; 16];
    block[1] = 0x03; // loop start + loop end
    for (i, b) in block[2..].iter_mut().enumerate() {
        *b = if i % 2 == 0 { 0x74 } else { 0x9B };
    }
    [block; 8].concat()
}

/// Builds a combined VAB with `programs_count` populated programs, each with
/// one full-range tone referencing VAG slot 1. Returns `(bytes, table_bytes)`.
fn build_vab(programs_count: u16) -> (Vec<u8>, usize) {
    let bodies = synthetic_adpcm();
    let table_bytes = 32 + 128 * 16 + (programs_count as usize) * 16 * 32 + 256 * 2;
    let mut out = vec![0u8; table_bytes + bodies.len()];
    out[0..4].copy_from_slice(b"pBAV");
    out[4..8].copy_from_slice(&7u32.to_le_bytes()); // version 7 (v5+)
    out[12..16].copy_from_slice(&((table_bytes + bodies.len()) as u32).to_le_bytes());
    out[18..20].copy_from_slice(&programs_count.to_le_bytes());
    out[20..22].copy_from_slice(&programs_count.to_le_bytes());
    out[22..24].copy_from_slice(&1u16.to_le_bytes()); // vag_count
    out[24] = 127; // master_volume
    out[25] = 64; // master_pan
    for p in 0..programs_count as usize {
        let off = 32 + p * 16;
        out[off] = 1; // 1 tone
        out[off + 1] = 127;
        out[off + 2] = 8; // priority
        out[off + 4] = 64;
    }
    for p in 0..programs_count as usize {
        let off = 32 + 128 * 16 + p * 16 * 32;
        out[off] = 8; // priority
        out[off + 2] = 127; // volume
        out[off + 3] = 64; // pan
        out[off + 4] = 60; // center
        out[off + 6] = 0; // min
        out[off + 7] = 127; // max
        out[off + 16..off + 18].copy_from_slice(&0x00FFu16.to_le_bytes());
        out[off + 18..off + 20].copy_from_slice(&0x000Fu16.to_le_bytes());
        out[off + 20..off + 22].copy_from_slice(&(p as i16).to_le_bytes());
        out[off + 22..off + 24].copy_from_slice(&1i16.to_le_bytes()); // vag 1
    }
    let sizes_off = 32 + 128 * 16 + (programs_count as usize) * 16 * 32;
    out[sizes_off + 2..sizes_off + 4].copy_from_slice(&((bodies.len() / 8) as u16).to_le_bytes());
    out[table_bytes..].copy_from_slice(&bodies);
    (out, table_bytes)
}

fn note_track(program: u8, key: u8, delta: u8) -> Vec<u8> {
    vec![
        0x00, // lead VLQ delta 0
        0xC7,
        0xFF, // Tempo(256), zero-delta
        0xC9,
        0x80 | program, // ProgramChange, zero-delta
        key,
        127,
        delta, // NoteOn, VLQ delta
        0xCB,  // RunningNoteOff, zero-delta
        0xFF,
        0x00, // EndOfTrack + flag byte
    ]
}

fn test_config() -> RenderConfig {
    RenderConfig {
        target_loops: 0,
        fade_seconds: 0.5,
        tail_seconds: 0.05,
        max_seconds: 5.0,
        ..RenderConfig::default()
    }
}

// ---------------------------------------------------------------------------
// Version / header / stream parsing
// ---------------------------------------------------------------------------

#[test]
fn version_detection_timer_and_geometry() {
    assert_eq!(detect_version(b"KDT ").unwrap(), KdtVersion::Kdt);
    assert_eq!(detect_version(b"KDT1").unwrap(), KdtVersion::Kdt1);
    assert_eq!(detect_version(b"MDT1").unwrap(), KdtVersion::Mdt1);
    assert!(detect_version(b"MDT2").is_err());
    assert!(detect_version(b"NOPE").is_err());
    assert!(detect_version(b"KD").is_err());
    assert_eq!(KdtVersion::Kdt.timer_divider(), 7328);
    assert!((KdtVersion::Kdt1.timer_hz() - 577.733).abs() < 0.01);
    assert_eq!(KdtVersion::Kdt.data_offset(4), 0x50);
    assert_eq!(KdtVersion::Kdt1.data_offset(4), 24);
    assert_eq!(KdtVersion::Mdt1.data_offset(1), 18);
}

#[test]
fn parses_all_three_variants_and_decodes_compact_events() {
    let track0: &[u8] = &[
        0x00, // lead delta 0
        0xC6, 0x82, // ChannelSelect(2), zero-delta
        0xC7, 0xFF, // Tempo(256), zero-delta
        0xC8, 0x81, // BankParam(1), zero-delta
        0xC9, 0x83, // ProgramChange(3), zero-delta
        0x87, 0xE4, // CC7 = 100, zero-delta
        60, 127, 24, // NoteOn(60, 127), delta 24
        0xCA, 12, // RunningNoteOff, delta 12
        64, 0xE4, // NoteOn(64, 100), zero-delta
        0xCB, // RunningNoteOff, zero-delta
        0xFF, 0x00, // EndOfTrack
    ];
    let expected = vec![
        KdtEvent {
            command: KdtCommand::ChannelSelect(2),
            delta: 0,
        },
        KdtEvent {
            command: KdtCommand::Tempo(256),
            delta: 0,
        },
        KdtEvent {
            command: KdtCommand::BankParam(1),
            delta: 0,
        },
        KdtEvent {
            command: KdtCommand::ProgramChange(3),
            delta: 0,
        },
        KdtEvent {
            command: KdtCommand::ControlChange {
                controller: 7,
                value: 100,
            },
            delta: 0,
        },
        KdtEvent {
            command: KdtCommand::NoteOn {
                key: 60,
                velocity: 127,
            },
            delta: 24,
        },
        KdtEvent {
            command: KdtCommand::RunningNoteOff,
            delta: 12,
        },
        KdtEvent {
            command: KdtCommand::NoteOn {
                key: 64,
                velocity: 100,
            },
            delta: 0,
        },
        KdtEvent {
            command: KdtCommand::RunningNoteOff,
            delta: 0,
        },
        KdtEvent {
            command: KdtCommand::EndOfTrack,
            delta: 0,
        },
    ];
    for (magic, version) in [
        (*b"KDT ", KdtVersion::Kdt),
        (*b"KDT1", KdtVersion::Kdt1),
        (*b"MDT1", KdtVersion::Mdt1),
    ] {
        let bytes = build_kdt_bytes(magic, 48, &[track0]);
        let view = KdtSequenceView::parse(&bytes, None).unwrap();
        assert_eq!(view.version, version);
        assert_eq!(view.ppqn, 48);
        assert_eq!(view.track_count(), 1);
        let track = view.track(0).unwrap();
        assert_eq!(track.lead_delta(), 0);
        let events: Vec<KdtEvent> = track.events().collect();
        assert_eq!(events, expected);
        let seq = KdtSequence::parse(&bytes).unwrap();
        assert_eq!(seq.version, version);
        assert_eq!(seq.tracks[0].events, expected);
        assert_eq!(seq.tracks[0].lead_delta, 0);
    }
}

#[test]
fn vlq_multibyte_delta_and_lead_delta() {
    // Lead delta 5, NoteOn with VLQ delta 300 (0x82 0x2C), explicit NoteOff.
    let track: &[u8] = &[0x05, 60, 127, 0x82, 0x2C, 60, 0x00, 0x00, 0xFF, 0x00];
    let bytes = build_kdt_bytes(*b"KDT1", 96, &[track]);
    let seq = KdtSequence::parse(&bytes).unwrap();
    assert_eq!(seq.tracks[0].lead_delta, 5);
    assert_eq!(
        seq.tracks[0].events,
        vec![
            KdtEvent {
                command: KdtCommand::NoteOn {
                    key: 60,
                    velocity: 127
                },
                delta: 300
            },
            KdtEvent {
                command: KdtCommand::NoteOff { key: 60 },
                delta: 0
            },
            KdtEvent {
                command: KdtCommand::EndOfTrack,
                delta: 0
            },
        ]
    );
}

#[test]
fn multi_track_offsets_lengths_and_empty_tracks() {
    let t0 = note_track(0, 60, 10);
    let t1 = note_track(1, 67, 8);
    let bytes = build_kdt_bytes(*b"MDT1", 48, &[&t0, &[], &t1]);
    let view = KdtSequenceView::parse(&bytes, None).unwrap();
    assert_eq!(view.track_count(), 3);
    assert_eq!(view.track_len(0), Some(t0.len() as u16));
    assert_eq!(view.track_len(1), Some(0));
    assert_eq!(view.track(1).unwrap().bytes.len(), 0);
    assert_eq!(view.track(2).unwrap().bytes, t1.as_slice());
    let seq = KdtSequence::parse(&bytes).unwrap();
    assert_eq!(seq.tracks.len(), 3);
    assert!(seq.tracks[1].events.is_empty());
    assert_eq!(seq.tracks[2].events.len(), 5);
}

// ---------------------------------------------------------------------------
// VAB bank
// ---------------------------------------------------------------------------

#[test]
fn vab_combined_split_views_and_upload() {
    let (bytes, table_bytes) = build_vab(2);
    let view = VabBankView::parse_vab(&bytes).unwrap();
    assert_eq!(view.header.program_count, 2);
    assert_eq!(view.header.master_volume, 127);
    assert_eq!(
        view.program(1),
        Some(ProgramAttr {
            tones: 1,
            master_volume: 127,
            priority: 8,
            mode: 0,
            master_pan: 64
        })
    );
    let tone = view.tone(1, 0).unwrap();
    assert_eq!(
        tone,
        ToneAttr {
            priority: 8,
            mode: 0,
            volume: 127,
            pan: 64,
            center: 60,
            shift: 0,
            min: 0,
            max: 127,
            adsr1: 0x00FF,
            adsr2: 0x000F,
            program: 1,
            vag: 1,
        }
    );
    assert_eq!(view.block_for_program(1), Some(1));
    assert_eq!(view.vag_body(1).unwrap().len(), synthetic_adpcm().len());
    assert!(view.vag_body(2).is_none());

    let bank = VabBank::parse_vab(&bytes).unwrap();
    let (prog, tones) = bank.program_block(1).unwrap();
    assert_eq!(prog.tones, 1);
    assert_eq!(tones[0].vag, 1);

    // Split .VH/.VB parses identically to combined.
    let split = VabBank::parse_vh_vb(&bytes[..table_bytes], &bytes[table_bytes..]).unwrap();
    assert_eq!(split, bank);

    // Upload lands at 0x1010 and RAM holds the ADPCM payload.
    let mut device = libspu2::Device::new_ps1();
    let upload = bank.upload_to_spu(&mut device).unwrap();
    assert_eq!(upload.base_addr, 0x1010);
    assert_eq!(upload.vag_addrs[1], Some(0x1010));
    assert_eq!(upload.vag_addrs[2], None);
    let mut ram = vec![0u8; 16];
    device.read_ram(0x1010, &mut ram).unwrap();
    assert_eq!(ram[1], 0x03);
}

#[test]
fn error_validation_rejects_malformed_inputs() {
    assert!(KdtSequence::parse(b"NOPE........").is_err());
    assert!(KdtSequence::parse(b"KDT1").is_err());
    let mut zero_tracks = build_kdt_bytes(*b"KDT1", 48, &[&note_track(0, 60, 4)]);
    zero_tracks[12..14].copy_from_slice(&0u16.to_le_bytes());
    assert!(KdtSequence::parse(&zero_tracks).is_err());
    let (vab_bytes, _) = build_vab(1);
    let mut bad_vab = vab_bytes.clone();
    bad_vab[0] = b'X';
    assert!(VabBank::parse_vab(&bad_vab).is_err());
    assert!(VabBank::parse_vab(&vab_bytes[..64]).is_err());

    let seq = KdtSequence::parse(&build_kdt_bytes(*b"KDT1", 48, &[&note_track(0, 60, 4)])).unwrap();
    let bank = VabBank::parse_vab(&vab_bytes).unwrap();
    let mut bad_cfg = test_config();
    bad_cfg.sample_rate = 0;
    assert!(KdtPlayer::new(seq.clone(), bank.clone(), bad_cfg).is_err());
    bad_cfg = test_config();
    bad_cfg.fade_seconds = -1.0;
    assert!(KdtPlayer::new(seq, bank, bad_cfg).is_err());
}

// ---------------------------------------------------------------------------
// Player end-to-end
// ---------------------------------------------------------------------------

#[test]
fn end_to_end_render_all_variants_audible() {
    let (vab_bytes, _) = build_vab(2);
    for magic in [*b"KDT ", *b"KDT1", *b"MDT1"] {
        let t0 = note_track(0, 60, 60);
        let t1 = note_track(1, 67, 60);
        let seq = KdtSequence::parse(&build_kdt_bytes(magic, 48, &[&t0, &t1])).unwrap();
        let bank = VabBank::parse_vab(&vab_bytes).unwrap();
        let mut player = KdtPlayer::new(seq, bank, test_config()).unwrap();
        let result = player.render_all().unwrap();
        assert!(player.is_finished());
        assert!(result.reached_end);
        assert_eq!(result.loops_completed, 1);
        let peak = result
            .samples
            .iter()
            .map(|&s| i32::from(s).abs())
            .max()
            .unwrap_or(0);
        assert!(
            peak > 500,
            "variant {magic:?} expected audible render, peak={peak}"
        );
    }
}

#[test]
fn streaming_f32_and_i16_paths_agree() {
    let (vab_bytes, _) = build_vab(1);
    let seq =
        KdtSequence::parse(&build_kdt_bytes(*b"KDT1", 48, &[&note_track(0, 60, 80)])).unwrap();
    let bank = VabBank::parse_vab(&vab_bytes).unwrap();

    let mut p1 = KdtPlayer::new(seq.clone(), bank.clone(), test_config()).unwrap();
    let mut i16_buf = vec![0i16; 44_100];
    let n1 = p1.render_samples(&mut i16_buf).unwrap();

    let mut p2 = KdtPlayer::new(seq.clone(), bank.clone(), test_config()).unwrap();
    let (mut l, mut r) = (vec![0.0f32; 22_050], vec![0.0f32; 22_050]);
    let n2 = p2.render_f32_planar(&mut l, &mut r).unwrap();
    assert_eq!(n1, n2);
    for i in 0..n2 {
        assert!((l[i] - f32::from(i16_buf[i * 2]) / 32768.0).abs() < 0.001);
        assert!((r[i] - f32::from(i16_buf[i * 2 + 1]) / 32768.0).abs() < 0.001);
    }

    let mut p3 = KdtPlayer::new(seq, bank, test_config()).unwrap();
    let mut inter = vec![0.0f32; 44_100];
    let n3 = p3.render_f32_interleaved(&mut inter).unwrap();
    assert_eq!(n3, n2);
    assert!(inter[..n3 * 2].iter().any(|&s| s != 0.0));
    assert!(p3.render_samples(&mut [0i16; 3]).is_err());
    assert!(p3.render_f32_planar(&mut [0.0; 4], &mut [0.0; 5]).is_err());
}

#[test]
fn per_track_tempo_channels_and_cc_state() {
    let (vab_bytes, _) = build_vab(2);
    // Track 0 -> channel 0 via 0x46, program 0, CC7=100; Track 1 default channel 1, program 1.
    let mut t0 = vec![0x00, 0xC6, 0x80, 0xC7, 0xFF, 0xC9, 0x80, 0x87, 0xE4];
    t0.extend_from_slice(&[60, 127, 40, 0xCB, 0xFF, 0x00]);
    let t1 = note_track(1, 67, 40);
    let seq = KdtSequence::parse(&build_kdt_bytes(*b"KDT1", 48, &[&t0, &t1])).unwrap();
    let bank = VabBank::parse_vab(&vab_bytes).unwrap();
    let mut player = KdtPlayer::new(seq, bank, test_config()).unwrap();
    let mut buf = vec![0i16; 4096];
    let _ = player.render_samples(&mut buf).unwrap();
    let s0 = player.channel_snapshot(0).unwrap();
    assert_eq!(s0.program, 0);
    assert_eq!(s0.volume, 100);
    assert_eq!(s0.key, Some(60));
    assert!(s0.voice.is_some());
    assert!(s0.voice_state.is_some());
    let s1 = player.channel_snapshot(1).unwrap();
    assert_eq!(s1.program, 1);
    assert_eq!(s1.key, Some(67));
    assert!(player.channel_snapshot(16).is_none());
}

#[test]
fn mute_solo_reset_and_elapsed() {
    let (vab_bytes, _) = build_vab(1);
    let seq =
        KdtSequence::parse(&build_kdt_bytes(*b"KDT1", 48, &[&note_track(0, 60, 200)])).unwrap();
    let bank = VabBank::parse_vab(&vab_bytes).unwrap();
    let mut player = KdtPlayer::new(seq, bank, test_config()).unwrap();
    player.set_channel_muted(0, true);
    assert!(player.channel_snapshot(0).unwrap().muted);
    assert_eq!(player.mute_mask(), 1);
    player.set_mute_mask(0);
    player.set_solo_mask(0b10);
    assert!(player.channel_snapshot(0).unwrap().muted); // solo excludes ch 0
    assert!(!player.channel_snapshot(1).unwrap().muted);
    player.set_solo_mask(0);

    let mut buf = vec![0i16; 4096];
    let _ = player.render_samples(&mut buf).unwrap();
    assert!(player.elapsed_seconds() > 0.0);
    assert!(player.frames_rendered() > 0);
    player.reset().unwrap();
    assert_eq!(player.frames_rendered(), 0);
    assert_eq!(player.elapsed_seconds(), 0.0);
    assert_eq!(player.loops_completed(), 0);
    assert!(!player.is_finished());
    let result = player.render_all().unwrap();
    assert!(result.samples.iter().any(|&s| s != 0));
}

#[test]
fn linear_fade_out_preserves_master_volume() {
    let (vab_bytes, _) = build_vab(1);
    let seq =
        KdtSequence::parse(&build_kdt_bytes(*b"KDT1", 48, &[&note_track(0, 60, 120)])).unwrap();
    let bank = VabBank::parse_vab(&vab_bytes).unwrap();
    let mut cfg = test_config();
    cfg.target_loops = 1;
    cfg.fade_seconds = 0.3;
    let mut player = KdtPlayer::new(seq, bank, cfg).unwrap();
    let result = player.render_all().unwrap();
    assert!(result.faded_out);
    assert!(player.is_finished());
    assert_eq!(player.sequence_volume(), 0);
    // Fade scales VP_VOLL/VP_VOLR only: MVOLL/MVOLR keep the configured value.
    let mvol = player.device().read_ps1_register(ps1::MVOLL).unwrap();
    assert_eq!(mvol, 0x3FFF);
}

#[test]
fn reverb_preset_programmed_and_overlap_guarded() {
    let (vab_bytes, _) = build_vab(1);
    assert_eq!(ReverbPreset::ALL.len(), 10);
    assert_eq!(ReverbPreset::from_index(5), Some(ReverbPreset::Hall));
    assert!(ReverbPreset::from_index(10).is_none());
    let seq =
        KdtSequence::parse(&build_kdt_bytes(*b"KDT1", 48, &[&note_track(0, 60, 40)])).unwrap();
    let bank = VabBank::parse_vab(&vab_bytes).unwrap();
    let mut cfg = test_config();
    cfg.reverb_preset = ReverbPreset::Hall;
    cfg.reverb_depth = 0x2000;
    let player = KdtPlayer::new(seq, bank, cfg).unwrap();
    let evoll = player.device().read_ps1_register(ps1::EVOLL).unwrap();
    assert_eq!(evoll, 0x2000);
    let d_apf1 = player.device().read_ps1_register(ps1::RVB_BASE).unwrap();
    assert_eq!(d_apf1, 0x01A5); // Hall dAPF1
}

#[test]
fn voice_allocator_handles_24_simultaneous_notes() {
    let (vab_bytes, _) = build_vab(1);
    // 16 tracks on 16 channels, each triggering a distinct long note at once,
    // plus a second wave overloading the 24-voice pool.
    let mut streams: Vec<Vec<u8>> = Vec::new();
    for ch in 0..16u8 {
        let mut s = vec![0x00, 0xC6, 0x80 | ch, 0xC7, 0xFF, 0xC9, 0x80];
        s.extend_from_slice(&[48 + ch, 127, 200, 48 + ch, 127, 200, 0xFF, 0x00]);
        streams.push(s);
    }
    let refs: Vec<&[u8]> = streams.iter().map(|s| s.as_slice()).collect();
    let seq = KdtSequence::parse(&build_kdt_bytes(*b"KDT1", 48, &refs)).unwrap();
    let bank = VabBank::parse_vab(&vab_bytes).unwrap();
    let mut player = KdtPlayer::new(seq, bank, test_config()).unwrap();
    let mut buf = vec![0i16; 16384];
    let _ = player.render_samples(&mut buf).unwrap();
    let with_voice = (0..16)
        .filter(|&ch| {
            player
                .channel_snapshot(ch)
                .is_some_and(|s| s.voice.is_some())
        })
        .count();
    assert!(
        with_voice >= 8,
        "expected many allocated voices, got {with_voice}"
    );
    let result = player.render_all().unwrap();
    assert!(result.samples.iter().any(|&s| s != 0));
}
