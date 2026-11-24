// SPDX-License-Identifier: MIT OR Apache-2.0

//! Konami KDT sequence variants, hardware timer rates, and version detection.
//!
//! The variant set and header geometry were verified against Ghidra
//! decompilations of the shared Konami Computer Entertainment (KCE) PS1 sound
//! driver found in *Suikoden II* (`SLUS-00958`, accepts `KDT `/`KDT1`) and
//! *Vandal Hearts II* (`SLUS-00940`, accepts `KDT `/`KDT1`/`MDT1` alongside
//! PsyQ `pSEQ` and Standard MIDI `MThd`). Both titles program Root Counter 2
//! with `SetRCnt(0xF2000002, 0x1CA0, 0x1000)`, i.e. target `7328` on the
//! `4,233,600 Hz` counter clock. `MDT2` appears only as a data-file extension
//! string in *Vandal Hearts II*; no decompiled loader accepts an `MDT2`
//! header magic, so it is not a distinct variant.

use crate::{Error, Result};

/// PlayStation 1 Root Counter 2 input clock (`SysClk / 8` = `4,233,600 Hz`).
pub const KDT_RC2_CLOCK_HZ: u32 = 4_233_600;

/// Root Counter 2 target divider programmed by the KCE driver (`0x1CA0`).
pub const KDT_RC2_DIVIDER: u32 = 7_328;

/// Sequencer hardware timer frequency in Hz (`4_233_600 / 7_328` ≈ `577.73`).
pub const KDT_TIMER_HZ: f64 = KDT_RC2_CLOCK_HZ as f64 / KDT_RC2_DIVIDER as f64;

/// Fixed KDT container header size before the track-length table (`16`).
pub const KDT_HEADER_BYTES: usize = 16;

/// Data start offset for the original `"KDT "` container (`0x50`).
pub const KDT_LEGACY_DATA_OFFSET: usize = 0x50;

/// Supported Konami PlayStation 1 KDT sequence container variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum KdtVersion {
    /// Original `"KDT "` container (fixed `0x50` data start).
    Kdt,
    /// `"KDT1"` container (compact `16 + 2 * track_count` header).
    Kdt1,
    /// `"MDT1"` container (compact header, otherwise identical to `KDT1`).
    Mdt1,
}

impl KdtVersion {
    /// Returns the 4-byte ASCII magic tag for this variant.
    #[inline]
    #[must_use]
    pub const fn magic(self) -> [u8; 4] {
        match self {
            Self::Kdt => *b"KDT ",
            Self::Kdt1 => *b"KDT1",
            Self::Mdt1 => *b"MDT1",
        }
    }

    /// Returns the canonical static variant tag string.
    #[inline]
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Kdt => "KDT ",
            Self::Kdt1 => "KDT1",
            Self::Mdt1 => "MDT1",
        }
    }

    /// Decodes a variant from its 4-byte header magic, or `None` if unknown.
    #[inline]
    #[must_use]
    pub const fn from_magic(magic: [u8; 4]) -> Option<Self> {
        match magic {
            [b'K', b'D', b'T', b' '] => Some(Self::Kdt),
            [b'K', b'D', b'T', b'1'] => Some(Self::Kdt1),
            [b'M', b'D', b'T', b'1'] => Some(Self::Mdt1),
            _ => None,
        }
    }

    /// Returns the byte offset at which concatenated track streams begin.
    ///
    /// `"KDT "` uses a fixed `0x50` data start; `"KDT1"`/`"MDT1"` place track
    /// data immediately after the `u16` track-length table.
    #[inline]
    #[must_use]
    pub const fn data_offset(self, track_count: u16) -> usize {
        match self {
            Self::Kdt => KDT_LEGACY_DATA_OFFSET,
            Self::Kdt1 | Self::Mdt1 => KDT_HEADER_BYTES + (track_count as usize) * 2,
        }
    }

    /// Returns the Root Counter 2 target divider for this variant (`7328`).
    #[inline]
    #[must_use]
    pub const fn timer_divider(self) -> u32 {
        KDT_RC2_DIVIDER
    }

    /// Returns the hardware sequencer timer frequency in Hz (≈ `577.73`).
    #[inline]
    #[must_use]
    pub const fn timer_hz(self) -> f64 {
        KDT_TIMER_HZ
    }
}

/// Attempts to auto-detect the [`KdtVersion`] of a raw KDT sequence buffer.
///
/// Detection is by exact 4-byte header magic only; structural validation is
/// performed by [`crate::KdtSequenceView::parse`].
pub fn detect_version(bytes: &[u8]) -> Result<KdtVersion> {
    if bytes.len() < 4 {
        return Err(Error::UnexpectedEof("KDT sequence magic"));
    }
    let magic: [u8; 4] = [bytes[0], bytes[1], bytes[2], bytes[3]];
    KdtVersion::from_magic(magic).ok_or(Error::UnknownVersion)
}
