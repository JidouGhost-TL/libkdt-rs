// SPDX-License-Identifier: MIT OR Apache-2.0

//! Konami KDT sequence header, track-length table, and compact track-stream parser.
//!
//! [`KdtSequenceView`] provides zero-allocation borrowed parsing usable in
//! `#![no_std]` environments without `alloc`. With the `alloc` feature
//! enabled, [`KdtSequence`] provides an owned, fully decoded representation.
//!
//! Header and opcode grammar:
//!
//! - Header (`16` bytes): magic, `data_size: u32le`, `ppqn: u16le`,
//!   `reserved0: u16le`, `track_count: u16le`, `reserved1: u16le`.
//! - A `u16le` per-track byte-length table follows at `+0x10`; track streams
//!   are concatenated starting at [`KdtVersion::data_offset`].
//! - Each track stream is `[VLQ lead delta] (event [VLQ delta])*`. An event's
//!   final parameter byte carries an MSB `0x80` zero-delta continuation flag:
//!   when set, the event's post-delta is `0` and no VLQ follows.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::version::{detect_version, KdtVersion, KDT_HEADER_BYTES};
use crate::{Error, Result};

/// Decoded command inside a KDT track event stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KdtCommand {
    /// Note-on (`velocity > 0`) for MIDI note `key`.
    NoteOn {
        /// MIDI note number (`0..=127`).
        key: u8,
        /// Note velocity (`1..=127`).
        velocity: u8,
    },
    /// Explicit note-off (`velocity == 0`) for MIDI note `key`.
    NoteOff {
        /// MIDI note number (`0..=127`).
        key: u8,
    },
    /// Select logical channel (`0x46`, masked to `0..=15`).
    ChannelSelect(u8),
    /// Set sequence tempo increment across all tracks (`0x47`, `(val & 0x7F) * 2 + 2`, range `2..=256`).
    Tempo(u16),
    /// Pitch bend (`0x48`, 7-bit `0..=127`, centered at `64`).
    PitchBend(u8),
    /// Program change (`0x49`).
    ProgramChange(u8),
    /// Running-note off (`0x4A` with VLQ delta / `0x4B` with zero delta).
    RunningNoteOff,
    /// Control change (`controller = cmd & 0x7F`, `value = param & 0x7F`).
    ControlChange {
        /// MIDI controller number (`0..=127`).
        controller: u8,
        /// Controller value (`0..=127`).
        value: u8,
    },
    /// End of track (`0x7F`, raw byte `0xFF`, one trailing flag byte consumed).
    EndOfTrack,
}

/// One decoded track event with its post-event tick delta (`0` when the MSB
/// `0x80` zero-delta continuation flag was set on the event's parameter byte).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct KdtEvent {
    /// Decoded command payload.
    pub command: KdtCommand,
    /// Musical ticks to wait after executing `command`.
    pub delta: u32,
}

/// Zero-allocation iterator decoding a compact KDT track stream.
///
/// The leading VLQ delta is consumed at construction and exposed via
/// [`TrackEventIter::lead_delta`]; each yielded [`KdtEvent`] carries its
/// post-event delta.
#[derive(Debug, Clone)]
pub struct TrackEventIter<'a> {
    stream: &'a [u8],
    pos: usize,
    lead_delta: u32,
    done: bool,
}

impl<'a> TrackEventIter<'a> {
    /// Creates an iterator over `stream`, consuming its leading VLQ delta.
    #[must_use]
    pub fn new(stream: &'a [u8]) -> Self {
        let mut iter = Self {
            stream,
            pos: 0,
            lead_delta: 0,
            done: false,
        };
        iter.lead_delta = iter.read_vlq();
        iter
    }

    /// Returns the track's leading VLQ delta (ticks before the first event).
    #[inline]
    #[must_use]
    pub const fn lead_delta(&self) -> u32 {
        self.lead_delta
    }

    fn read_vlq(&mut self) -> u32 {
        let mut value: u32 = 0;
        for _ in 0..5 {
            let Some(&b) = self.stream.get(self.pos) else {
                break;
            };
            self.pos += 1;
            value = (value << 7) | u32::from(b & 0x7F);
            if (b & 0x80) == 0 {
                break;
            }
        }
        value
    }

    fn next_byte(&mut self) -> Option<u8> {
        let b = *self.stream.get(self.pos)?;
        self.pos += 1;
        Some(b)
    }
}

impl Iterator for TrackEventIter<'_> {
    type Item = KdtEvent;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let b = self.next_byte()?;
        if b < 0x80 {
            let key = b & 0x7F;
            let vel_byte = self.next_byte()?;
            let velocity = vel_byte & 0x7F;
            let delta = if (vel_byte & 0x80) != 0 {
                0
            } else {
                self.read_vlq()
            };
            let command = if velocity == 0 {
                KdtCommand::NoteOff { key }
            } else {
                KdtCommand::NoteOn { key, velocity }
            };
            return Some(KdtEvent { command, delta });
        }
        match b & 0x7F {
            0x7F => {
                // Hardware consumes one trailing flag byte after end-of-track.
                let _ = self.next_byte();
                self.done = true;
                Some(KdtEvent {
                    command: KdtCommand::EndOfTrack,
                    delta: 0,
                })
            }
            0x4A => Some(KdtEvent {
                command: KdtCommand::RunningNoteOff,
                delta: self.read_vlq(),
            }),
            0x4B => Some(KdtEvent {
                command: KdtCommand::RunningNoteOff,
                delta: 0,
            }),
            cmd @ 0x46..=0x49 => {
                let param = self.next_byte()?;
                let delta = if (param & 0x80) != 0 {
                    0
                } else {
                    self.read_vlq()
                };
                let val = param & 0x7F;
                let command = match cmd {
                    0x46 => KdtCommand::ChannelSelect(val & 0x0F),
                    0x47 => KdtCommand::Tempo(u16::from(val) * 2 + 2),
                    0x48 => KdtCommand::PitchBend(val),
                    _ => KdtCommand::ProgramChange(val),
                };
                Some(KdtEvent { command, delta })
            }
            cc => {
                let param = self.next_byte()?;
                let delta = if (param & 0x80) != 0 {
                    0
                } else {
                    self.read_vlq()
                };
                Some(KdtEvent {
                    command: KdtCommand::ControlChange {
                        controller: cc,
                        value: param & 0x7F,
                    },
                    delta,
                })
            }
        }
    }
}

/// Decodes a full track stream into owned events (requires `alloc`).
#[cfg(feature = "alloc")]
#[must_use]
pub fn decode_track_stream(stream: &[u8]) -> (u32, Vec<KdtEvent>) {
    let iter = TrackEventIter::new(stream);
    let lead = iter.lead_delta();
    (lead, iter.collect())
}

/// Borrowed view of a single KDT track stream within a [`KdtSequenceView`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdtTrackView<'a> {
    /// Absolute byte offset of this track's stream in the source buffer.
    pub offset: u32,
    /// Raw compact track stream bytes.
    pub bytes: &'a [u8],
}

impl<'a> KdtTrackView<'a> {
    /// Returns the track's leading VLQ delta (ticks before the first event).
    #[must_use]
    pub fn lead_delta(&self) -> u32 {
        TrackEventIter::new(self.bytes).lead_delta()
    }

    /// Returns a zero-allocation decoding iterator over this track's events.
    #[must_use]
    pub fn events(&self) -> TrackEventIter<'a> {
        TrackEventIter::new(self.bytes)
    }
}

/// Zero-allocation borrowed view of a parsed Konami KDT sequence.
///
/// Usable in pure `#![no_std]` builds without the `alloc` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KdtSequenceView<'a> {
    /// Detected header magic variant.
    pub version: KdtVersion,
    /// Header `+0x04` `u32le` total data size / end offset.
    pub data_size: u32,
    /// Header `+0x08` `u16le` PPQN / timebase.
    pub ppqn: u16,
    /// Header `+0x0A` `u16le` reserved / flags word.
    pub reserved0: u16,
    /// Header `+0x0C` `u16le` declared track count.
    pub track_count: u16,
    /// Header `+0x0E` `u16le` reserved word.
    pub reserved1: u16,
    data: &'a [u8],
    data_offset: usize,
    data_limit: usize,
}

impl<'a> KdtSequenceView<'a> {
    /// Parses a borrowed KDT sequence view from `bytes` without allocating.
    ///
    /// Pass `Some(version)` to force a specific [`KdtVersion`], or `None` to
    /// auto-detect via [`detect_version`].
    pub fn parse(bytes: &'a [u8], version: Option<KdtVersion>) -> Result<Self> {
        let version = match version {
            Some(v) => v,
            None => detect_version(bytes)?,
        };
        if bytes.len() < KDT_HEADER_BYTES {
            return Err(Error::UnexpectedEof("KDT sequence header"));
        }
        let magic: [u8; 4] = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if KdtVersion::from_magic(magic) != Some(version) {
            return Err(Error::InvalidMagic);
        }
        let data_size = read_u32_le(bytes, 0x04);
        let ppqn = read_u16_le(bytes, 0x08);
        let reserved0 = read_u16_le(bytes, 0x0A);
        let track_count = read_u16_le(bytes, 0x0C);
        let reserved1 = read_u16_le(bytes, 0x0E);
        if track_count == 0 {
            return Err(Error::InvalidHeader("track_count must be non-zero"));
        }
        let table_end = KDT_HEADER_BYTES
            .checked_add((track_count as usize) * 2)
            .ok_or(Error::OutOfRange("track length table size overflow"))?;
        if table_end > bytes.len() {
            return Err(Error::UnexpectedEof("KDT track length table"));
        }
        let data_offset = version.data_offset(track_count);
        if data_offset < table_end {
            return Err(Error::InvalidHeader(
                "track length table exceeds legacy KDT data offset",
            ));
        }
        let data_limit =
            if (data_size as usize) >= data_offset && (data_size as usize) <= bytes.len() {
                data_size as usize
            } else {
                bytes.len()
            };
        // Validate cumulative track extents.
        let mut cursor = data_offset;
        for i in 0..track_count as usize {
            let len = read_u16_le(bytes, KDT_HEADER_BYTES + i * 2) as usize;
            cursor = cursor
                .checked_add(len)
                .ok_or(Error::OutOfRange("track extent overflow"))?;
            if cursor > data_limit {
                return Err(Error::OutOfRange("track extent exceeds data size"));
            }
        }
        Ok(Self {
            version,
            data_size,
            ppqn,
            reserved0,
            track_count,
            reserved1,
            data: bytes,
            data_offset,
            data_limit,
        })
    }

    /// Returns the number of declared tracks.
    #[inline]
    #[must_use]
    pub const fn track_count(&self) -> u16 {
        self.track_count
    }

    /// Returns the declared byte length of track `index`, or `None` if out of range.
    #[must_use]
    pub fn track_len(&self, index: usize) -> Option<u16> {
        if index >= self.track_count as usize {
            return None;
        }
        Some(read_u16_le(self.data, KDT_HEADER_BYTES + index * 2))
    }

    /// Returns the borrowed stream for track `index`, or `None` if out of range.
    #[must_use]
    pub fn track(&self, index: usize) -> Option<KdtTrackView<'a>> {
        if index >= self.track_count as usize {
            return None;
        }
        let mut start = self.data_offset;
        for i in 0..index {
            start += read_u16_le(self.data, KDT_HEADER_BYTES + i * 2) as usize;
        }
        let len = read_u16_le(self.data, KDT_HEADER_BYTES + index * 2) as usize;
        let end = (start + len).min(self.data_limit);
        Some(KdtTrackView {
            offset: start as u32,
            bytes: &self.data[start..end],
        })
    }

    /// Returns the underlying raw sequence slice.
    #[inline]
    #[must_use]
    pub const fn data(&self) -> &'a [u8] {
        self.data
    }

    /// Converts this borrowed view into an owned [`KdtSequence`].
    #[cfg(feature = "alloc")]
    #[must_use]
    pub fn to_owned_sequence(&self) -> KdtSequence {
        let mut tracks = Vec::with_capacity(self.track_count as usize);
        for i in 0..self.track_count as usize {
            if let Some(view) = self.track(i) {
                let (lead_delta, events) = decode_track_stream(view.bytes);
                tracks.push(KdtTrack {
                    offset: view.offset,
                    raw: view.bytes.to_vec(),
                    lead_delta,
                    events,
                });
            }
        }
        KdtSequence {
            version: self.version,
            data_size: self.data_size,
            ppqn: self.ppqn,
            reserved0: self.reserved0,
            track_count: self.track_count,
            reserved1: self.reserved1,
            tracks,
        }
    }
}

/// A single owned, decoded track within a [`KdtSequence`].
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KdtTrack {
    /// Absolute byte offset of this track's stream in the source buffer.
    pub offset: u32,
    /// Raw compact track stream bytes.
    pub raw: Vec<u8>,
    /// Leading VLQ delta (ticks before the first event).
    pub lead_delta: u32,
    /// Decoded compact MIDI-like track events.
    pub events: Vec<KdtEvent>,
}

/// Owned parsed Konami `"KDT "` / `"KDT1"` / `"MDT1"` multi-track sequence.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KdtSequence {
    /// Detected header magic variant.
    pub version: KdtVersion,
    /// Header `+0x04` `u32le` total data size / end offset.
    pub data_size: u32,
    /// Header `+0x08` `u16le` PPQN / timebase.
    pub ppqn: u16,
    /// Header `+0x0A` `u16le` reserved / flags word.
    pub reserved0: u16,
    /// Header `+0x0C` `u16le` declared track count.
    pub track_count: u16,
    /// Header `+0x0E` `u16le` reserved word.
    pub reserved1: u16,
    /// Parsed tracks in table order.
    pub tracks: Vec<KdtTrack>,
}

#[cfg(feature = "alloc")]
impl KdtSequence {
    /// Parses a KDT sequence from `bytes`, auto-detecting its variant.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        let view = KdtSequenceView::parse(bytes, None)?;
        Ok(view.to_owned_sequence())
    }

    /// Parses a KDT sequence from `bytes` with a forced variant.
    pub fn parse_with_version(bytes: &[u8], version: KdtVersion) -> Result<Self> {
        let view = KdtSequenceView::parse(bytes, Some(version))?;
        Ok(view.to_owned_sequence())
    }
}

#[inline]
fn read_u16_le(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

#[inline]
fn read_u32_le(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

/// Normalizes a raw KDT VLQ tick delta from header `ppqn` to the internal
/// 120 PPQN timebase using a per-track fractional remainder accumulator (`rem`).
///
/// Scaling ratios for standard KCE PlayStation 1 PPQN values:
/// - `48` (`0x30`): `(delta * 10 + rem) >> 2`, `rem = & 3`
/// - `96` (`0x60`): `(delta * 5 + rem) >> 2`, `rem = & 3`
/// - `192` (`0xC0`) / `240` (`0xF0`): `(delta + rem) >> 1`, `rem = & 1`
/// - `288` (`0x120`) / `360` (`0x168`): `(delta + rem) / 3`, `rem = % 3`
/// - `384` (`0x180`) / `480` (`0x1E0`): `(delta + rem) >> 2`, `rem = & 3`
/// - `768` (`0x300`) / `960` (`0x3C0`): `(delta + rem) >> 3`, `rem = & 7`
/// - `120` or any other value: `delta` unchanged.
#[inline]
#[must_use]
pub fn normalize_ppqn_delta(ppqn: u16, delta: u32, rem: &mut u32) -> u32 {
    match ppqn {
        48 => {
            let v = u64::from(delta) * 10 + u64::from(*rem);
            *rem = (v & 3) as u32;
            (v >> 2).min(u64::from(u32::MAX)) as u32
        }
        96 => {
            let v = u64::from(delta) * 5 + u64::from(*rem);
            *rem = (v & 3) as u32;
            (v >> 2).min(u64::from(u32::MAX)) as u32
        }
        192 | 240 => {
            let v = u64::from(delta) + u64::from(*rem);
            *rem = (v & 1) as u32;
            (v >> 1) as u32
        }
        288 | 360 => {
            let v = u64::from(delta) + u64::from(*rem);
            *rem = (v % 3) as u32;
            (v / 3) as u32
        }
        384 | 480 => {
            let v = u64::from(delta) + u64::from(*rem);
            *rem = (v & 3) as u32;
            (v >> 2) as u32
        }
        768 | 960 => {
            let v = u64::from(delta) + u64::from(*rem);
            *rem = (v & 7) as u32;
            (v >> 3) as u32
        }
        _ => delta,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_std_normalize_ppqn_delta_exact_ratios_and_remainders() {
        // PPQN 48: 48 raw ticks -> 120 normalized ticks; fractional 1-tick steps accumulate cleanly.
        let mut rem = 0;
        assert_eq!(normalize_ppqn_delta(48, 48, &mut rem), 120);
        assert_eq!(rem, 0);
        let mut sum = 0;
        for _ in 0..4 {
            sum += normalize_ppqn_delta(48, 1, &mut rem);
        }
        assert_eq!(sum, 10); // 4 * 2.5 = 10
        assert_eq!(rem, 0);

        // PPQN 96: 96 -> 120; four 1-tick steps -> 5 ticks.
        rem = 0;
        assert_eq!(normalize_ppqn_delta(96, 96, &mut rem), 120);
        sum = 0;
        for _ in 0..4 {
            sum += normalize_ppqn_delta(96, 1, &mut rem);
        }
        assert_eq!(sum, 5);
        assert_eq!(rem, 0);

        // PPQN 192 / 240: >> 1 with remainder.
        rem = 0;
        assert_eq!(normalize_ppqn_delta(240, 1, &mut rem), 0);
        assert_eq!(rem, 1);
        assert_eq!(normalize_ppqn_delta(240, 1, &mut rem), 1);
        assert_eq!(rem, 0);
        assert_eq!(normalize_ppqn_delta(192, 240, &mut rem), 120);

        // PPQN 288 / 360: / 3 with remainder.
        rem = 0;
        assert_eq!(normalize_ppqn_delta(360, 1, &mut rem), 0);
        assert_eq!(normalize_ppqn_delta(360, 1, &mut rem), 0);
        assert_eq!(normalize_ppqn_delta(360, 1, &mut rem), 1);
        assert_eq!(rem, 0);
        assert_eq!(normalize_ppqn_delta(288, 360, &mut rem), 120);

        // PPQN 384 / 480: >> 2 with remainder.
        rem = 0;
        assert_eq!(normalize_ppqn_delta(480, 480, &mut rem), 120);
        assert_eq!(normalize_ppqn_delta(384, 3, &mut rem), 0);
        assert_eq!(rem, 3);
        assert_eq!(normalize_ppqn_delta(384, 1, &mut rem), 1);
        assert_eq!(rem, 0);

        // PPQN 768 / 960: >> 3 with remainder.
        rem = 0;
        assert_eq!(normalize_ppqn_delta(960, 960, &mut rem), 120);
        assert_eq!(normalize_ppqn_delta(768, 7, &mut rem), 0);
        assert_eq!(rem, 7);
        assert_eq!(normalize_ppqn_delta(768, 1, &mut rem), 1);
        assert_eq!(rem, 0);

        // PPQN 120 and unknown pass through unchanged.
        rem = 0;
        assert_eq!(normalize_ppqn_delta(120, 37, &mut rem), 37);
        assert_eq!(normalize_ppqn_delta(60, 37, &mut rem), 37);
    }

    #[test]
    fn no_std_sequence_view_and_pitch_bend_decode() {
        // Minimal KDT1 with 1 track: lead=0, PitchBend(96) zero-delta, NoteOn(60,100) delta=12, EndOfTrack.
        const TRACK: [u8; 8] = [0x00, 0xC8, 0xE0, 60, 100, 12, 0xFF, 0x00];
        let mut buf = [0u8; 18 + TRACK.len()];
        let buf_len = buf.len() as u32;
        buf[0..4].copy_from_slice(b"KDT1");
        buf[4..8].copy_from_slice(&buf_len.to_le_bytes());
        buf[8..10].copy_from_slice(&48u16.to_le_bytes());
        buf[12..14].copy_from_slice(&1u16.to_le_bytes());
        buf[16..18].copy_from_slice(&(TRACK.len() as u16).to_le_bytes());
        buf[18..].copy_from_slice(&TRACK);

        let view = KdtSequenceView::parse(&buf, None).unwrap();
        assert_eq!(view.version, KdtVersion::Kdt1);
        assert_eq!(view.ppqn, 48);
        assert_eq!(view.track_count(), 1);
        let track = view.track(0).unwrap();
        assert_eq!(track.lead_delta(), 0);
        let mut iter = track.events();
        assert_eq!(
            iter.next(),
            Some(KdtEvent {
                command: KdtCommand::PitchBend(96),
                delta: 0
            })
        );
        assert_eq!(
            iter.next(),
            Some(KdtEvent {
                command: KdtCommand::NoteOn {
                    key: 60,
                    velocity: 100
                },
                delta: 12
            })
        );
        assert_eq!(
            iter.next(),
            Some(KdtEvent {
                command: KdtCommand::EndOfTrack,
                delta: 0
            })
        );
        assert_eq!(iter.next(), None);
    }
}
