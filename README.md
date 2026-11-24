# libkdt

`libkdt` is a standalone, `#![no_std]` (`alloc`-compatible), `#![forbid(unsafe_code)]`
pure-Rust Konami PlayStation 1 **KDT / KDT1 / MDT1** music sequencer built
directly on top of [`libspu2`](https://github.com/JidouGhost-TL/libspu2-rs),
with a built-in minimal Sony **VAB** (`.VH` + `.VB`) bank loader so it is
100% self-contained.

## Supported Container Variants

| Variant | Magic | Header | Track table | Data start |
| :--- | :--- | :--- | :--- | :--- |
| **`Kdt`** | `"KDT "` | `16` B | `u16le` per-track byte lengths at `+0x10` | Fixed `0x50` |
| **`Kdt1`** | `"KDT1"` | `16` B | `u16le` per-track byte lengths at `+0x10` | `16 + 2 * track_count` |
| **`Mdt1`** | `"MDT1"` | `16` B | `u16le` per-track byte lengths at `+0x10` | `16 + 2 * track_count` |

Header fields: `data_size: u32le` (`+0x04`), `ppqn: u16le` (`+0x08`),
`track_count: u16le` (`+0x0C`). Track streams are compact MIDI-like:
`[VLQ lead delta] (event [VLQ delta])*`, where an event's final parameter
byte carries an MSB `0x80` zero-delta continuation flag. Opcodes: notes
(`< 0x80`), channel select `0x46`, global tempo `0x47` (`(val & 0x7F) * 2 + 2`),
pitch bend `0x48` (7-bit centered at `64`), program `0x49`, running-note off
`0x4A`/`0x4B`, CC pass-through (including NRPN loop markers `CC 99` + `CC 6`),
end-of-track `0x7F` (raw `0xFF`).

The KCE PS1 sound driver accepts `"KDT "`, `"KDT1"`, and `"MDT1"` sequence
containers and programs Root Counter 2 with target `0x1CA0` (`7328`) on the
`4,233,600 Hz` counter clock (`SysClk / 8`).

## Hardware-Faithful Playback

- **Root Counter 2 timing**: Exact rational `4,233,600 / 7,328 ≈ 577.73 Hz`
  timer with per-track 8-bit fractional tempo accumulators
  (`acc += tempo; if acc > 0xFF { acc -= 0x100; tick }`).
- **24-voice priority allocation** over PS1 SPU voices with VAB tone
  key-ranges, pitch, ADSR, and reverb-send (`mode & 0x04`) handling.
- **10 PlayStation SPU hardware reverb presets** with RAM-overlap guarding.
- **Per-sequence voice-volume fade-out**: 60 Hz linear ramp applied to each
  active voice's `VP_VOLL`/`VP_VOLR` registers, leaving `MVOLL`/`MVOLR`
  untouched so reverb tails and release envelopes decay naturally.
- Channel mute/solo masks, live `ChannelSnapshot` (with `libspu2::VoiceState`),
  `reset()`, `elapsed_seconds()`, and `render_samples` /
  `render_f32_planar` / `render_f32_interleaved` streaming renders.

## Quick Example

```rust
use libkdt::{KdtPlayer, KdtSequence, RenderConfig, VabBank};

// Build a minimal 1-track KDT1 sequence (tempo 256, program 0, one note).
let track: &[u8] = &[0x00, 0xC7, 0xFF, 0xC9, 0x80, 60, 127, 40, 0xCB, 0xFF, 0x00];
let mut kdt = vec![0u8; 18 + track.len()];
let kdt_len = kdt.len() as u32;
kdt[0..4].copy_from_slice(b"KDT1");
kdt[4..8].copy_from_slice(&kdt_len.to_le_bytes());
kdt[8..10].copy_from_slice(&48u16.to_le_bytes());
kdt[12..14].copy_from_slice(&1u16.to_le_bytes());
kdt[16..18].copy_from_slice(&(track.len() as u16).to_le_bytes());
kdt[18..].copy_from_slice(track);
let seq = KdtSequence::parse(&kdt).unwrap();

// Build a minimal VAB: 1 program, 1 full-range tone, 1 looping ADPCM body.
let mut adpcm = vec![0u8; 16];
adpcm[1] = 0x03;
for (i, b) in adpcm[2..].iter_mut().enumerate() { *b = if i % 2 == 0 { 0x74 } else { 0x9B }; }
let bodies = adpcm.repeat(8);
let tables = 32 + 128 * 16 + 16 * 32 + 256 * 2;
let mut vh = vec![0u8; tables];
vh[0..4].copy_from_slice(b"pBAV");
vh[4..8].copy_from_slice(&7u32.to_le_bytes());
vh[18..20].copy_from_slice(&1u16.to_le_bytes());
vh[22..24].copy_from_slice(&1u16.to_le_bytes());
vh[24] = 127;
vh[32] = 1; vh[33] = 127; vh[36] = 64;
let t = 32 + 128 * 16;
vh[t + 2] = 127; vh[t + 3] = 64; vh[t + 4] = 60; vh[t + 7] = 127;
vh[t + 16..t + 18].copy_from_slice(&0x00FFu16.to_le_bytes());
vh[t + 22..t + 24].copy_from_slice(&1i16.to_le_bytes());
vh[t + 16 * 32 + 2..t + 16 * 32 + 4].copy_from_slice(&((bodies.len() / 8) as u16).to_le_bytes());
let bank = VabBank::parse_vh_vb(&vh, &bodies).unwrap();

let mut cfg = RenderConfig::default();
cfg.tail_seconds = 0.05;
cfg.max_seconds = 1.0;

let mut player = KdtPlayer::new(seq, bank, cfg).unwrap();
let rendered = player.render_all().unwrap();
assert!(rendered.samples.iter().any(|&s| s != 0));
```

## License

Dual-licensed under MIT (`LICENSE-MIT`) or Apache-2.0 (`LICENSE-APACHE`), at your option.
