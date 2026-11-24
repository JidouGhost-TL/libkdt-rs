// SPDX-License-Identifier: MIT OR Apache-2.0

//! Audio rendering throughput benchmark for `libkdt`.

use std::hint::black_box;
use std::time::Instant;

use libkdt::{KdtPlayer, KdtSequence, RenderConfig, ReverbPreset, VabBank};

fn build_vab() -> VabBank {
    let mut block = [0u8; 16];
    block[1] = 0x03;
    for (i, b) in block[2..].iter_mut().enumerate() {
        *b = if i % 2 == 0 { 0x74 } else { 0x9B };
    }
    let bodies = [block; 8].concat();
    let table_bytes = 32 + 128 * 16 + 16 * 32 + 256 * 2;
    let mut out = vec![0u8; table_bytes + bodies.len()];
    out[0..4].copy_from_slice(b"pBAV");
    out[4..8].copy_from_slice(&7u32.to_le_bytes());
    out[12..16].copy_from_slice(&((table_bytes + bodies.len()) as u32).to_le_bytes());
    out[18..20].copy_from_slice(&1u16.to_le_bytes());
    out[20..22].copy_from_slice(&1u16.to_le_bytes());
    out[22..24].copy_from_slice(&1u16.to_le_bytes());
    out[24] = 127;
    out[25] = 64;
    out[32] = 1;
    out[33] = 127;
    out[34] = 8;
    out[36] = 64;
    let tone_off = 32 + 128 * 16;
    out[tone_off] = 8;
    out[tone_off + 1] = 0x04; // reverb send
    out[tone_off + 2] = 127;
    out[tone_off + 3] = 64;
    out[tone_off + 4] = 60;
    out[tone_off + 7] = 127;
    out[tone_off + 16..tone_off + 18].copy_from_slice(&0x00FFu16.to_le_bytes());
    out[tone_off + 18..tone_off + 20].copy_from_slice(&0x000Fu16.to_le_bytes());
    out[tone_off + 22..tone_off + 24].copy_from_slice(&1i16.to_le_bytes());
    let sizes_off = tone_off + 16 * 32;
    out[sizes_off + 2..sizes_off + 4].copy_from_slice(&((bodies.len() / 8) as u16).to_le_bytes());
    out[table_bytes..].copy_from_slice(&bodies);
    VabBank::parse_vab(&out).unwrap()
}

fn build_seq() -> KdtSequence {
    // 8 tracks: looping arpeggio (player restarts on completion).
    let mut streams: Vec<Vec<u8>> = Vec::new();
    for ch in 0..8u8 {
        let mut s = vec![0x00, 0xC6, 0x80 | ch, 0xC7, 0xFF, 0xC9, 0x80];
        for key in [60u8, 64, 67, 72] {
            s.extend_from_slice(&[key, 127, 12, 0xCB]);
        }
        s.extend_from_slice(&[0xFF, 0x00]);
        streams.push(s);
    }
    let data_offset = 16 + streams.len() * 2;
    let total = data_offset + streams.iter().map(|s| s.len()).sum::<usize>();
    let mut out = vec![0u8; total];
    out[0..4].copy_from_slice(b"KDT1");
    out[4..8].copy_from_slice(&(total as u32).to_le_bytes());
    out[8..10].copy_from_slice(&48u16.to_le_bytes());
    out[12..14].copy_from_slice(&(streams.len() as u16).to_le_bytes());
    for (i, s) in streams.iter().enumerate() {
        out[16 + i * 2..18 + i * 2].copy_from_slice(&(s.len() as u16).to_le_bytes());
    }
    let mut cur = data_offset;
    for s in &streams {
        out[cur..cur + s.len()].copy_from_slice(s);
        cur += s.len();
    }
    KdtSequence::parse(&out).unwrap()
}

fn main() {
    const TOTAL_FRAMES: usize = 44_100 * 10; // 10 seconds of audio @ 44.1 kHz
    const CHUNK: usize = 1024;

    let seq = build_seq();
    let bank = build_vab();
    let cfg = RenderConfig {
        target_loops: 0,
        max_seconds: 600.0,
        reverb_preset: ReverbPreset::Hall,
        ..RenderConfig::bgm_default()
    };

    // 1. Interleaved i16
    let mut player = KdtPlayer::new(seq.clone(), bank.clone(), cfg).unwrap();
    let mut i16_buf = vec![0i16; CHUNK * 2];
    let t0 = Instant::now();
    let mut done = 0usize;
    while done < TOTAL_FRAMES {
        let n = (TOTAL_FRAMES - done).min(CHUNK);
        let got = player.render_samples(&mut i16_buf[..n * 2]).unwrap();
        black_box(&i16_buf[..got * 2]);
        if got == 0 {
            player.reset().unwrap();
        }
        done += got;
    }
    let dt_i16 = t0.elapsed().as_secs_f64();
    let rt_i16 = (TOTAL_FRAMES as f64 / 44_100.0) / dt_i16;

    // 2. Planar f32 (WebAudio)
    player.reset().unwrap();
    let mut left = vec![0.0f32; CHUNK];
    let mut right = vec![0.0f32; CHUNK];
    let t1 = Instant::now();
    let mut done = 0usize;
    while done < TOTAL_FRAMES {
        let n = (TOTAL_FRAMES - done).min(CHUNK);
        let got = player
            .render_f32_planar(&mut left[..n], &mut right[..n])
            .unwrap();
        black_box(&left[..got]);
        black_box(&right[..got]);
        if got == 0 {
            player.reset().unwrap();
        }
        done += got;
    }
    let dt_planar = t1.elapsed().as_secs_f64();
    let rt_planar = (TOTAL_FRAMES as f64 / 44_100.0) / dt_planar;

    // 3. Interleaved f32
    player.reset().unwrap();
    let mut f32_buf = vec![0.0f32; CHUNK * 2];
    let t2 = Instant::now();
    let mut done = 0usize;
    while done < TOTAL_FRAMES {
        let n = (TOTAL_FRAMES - done).min(CHUNK);
        let got = player
            .render_f32_interleaved(&mut f32_buf[..n * 2])
            .unwrap();
        black_box(&f32_buf[..got * 2]);
        if got == 0 {
            player.reset().unwrap();
        }
        done += got;
    }
    let dt_inter = t2.elapsed().as_secs_f64();
    let rt_inter = (TOTAL_FRAMES as f64 / 44_100.0) / dt_inter;

    println!(
        "bench_audio (8-track KDT1 + Hall reverb, 10s @ 44.1kHz = {TOTAL_FRAMES} frames):\n\
         - render_samples (i16):         {:.2} ms ({:.1}x realtime, {:.2} MSamples/s)\n\
         - render_f32_planar (f32):      {:.2} ms ({:.1}x realtime, {:.2} MSamples/s)\n\
         - render_f32_interleaved (f32): {:.2} ms ({:.1}x realtime, {:.2} MSamples/s)",
        dt_i16 * 1000.0,
        rt_i16,
        (TOTAL_FRAMES as f64 * 2.0 / dt_i16) / 1e6,
        dt_planar * 1000.0,
        rt_planar,
        (TOTAL_FRAMES as f64 * 2.0 / dt_planar) / 1e6,
        dt_inter * 1000.0,
        rt_inter,
        (TOTAL_FRAMES as f64 * 2.0 / dt_inter) / 1e6,
    );
}
