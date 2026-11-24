// SPDX-License-Identifier: MIT OR Apache-2.0

#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![forbid(unsafe_code)]
#![warn(missing_docs)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::fmt;

pub mod bank;
#[cfg(feature = "alloc")]
pub mod player;
pub mod reverb;
pub mod sequence;
pub mod version;

pub use bank::{ProgramAttr, ToneAttr, VabBankView, VabHeader};
#[cfg(feature = "alloc")]
pub use bank::{VabBank, VabUpload};
#[cfg(feature = "alloc")]
pub use player::{ChannelSnapshot, KdtPlayer, RenderConfig, RenderResult};
pub use reverb::{ReverbParams, ReverbPreset};
pub use sequence::{KdtCommand, KdtEvent, KdtSequenceView, KdtTrackView, TrackEventIter};
#[cfg(feature = "alloc")]
pub use sequence::{KdtSequence, KdtTrack};
pub use version::{detect_version, KdtVersion, KDT_RC2_CLOCK_HZ, KDT_RC2_DIVIDER, KDT_TIMER_HZ};

/// Crate-wide result type alias for `libkdt`.
pub type Result<T> = core::result::Result<T, Error>;

/// Errors returned by `libkdt` parsing, upload, and playback APIs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Error {
    /// Input buffer was shorter than required for the header, table, or operand.
    UnexpectedEof(&'static str),
    /// Input did not begin with a recognized `KDT `/`KDT1`/`MDT1` or `VABp` magic.
    InvalidMagic,
    /// Header field violated a structural invariant.
    InvalidHeader(&'static str),
    /// Offset, table pointer, or SPU RAM address was out of range.
    OutOfRange(&'static str),
    /// Automatic KDT version detection could not match any supported variant.
    UnknownVersion,
    /// An invalid configuration or argument was supplied.
    InvalidArgument(&'static str),
    /// An underlying `libspu2` device error occurred.
    Spu(libspu2::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnexpectedEof(ctx) => write!(f, "unexpected end of data: {ctx}"),
            Self::InvalidMagic => write!(f, "invalid KDT/VAB magic header"),
            Self::InvalidHeader(msg) => write!(f, "invalid header: {msg}"),
            Self::OutOfRange(msg) => write!(f, "out of range: {msg}"),
            Self::UnknownVersion => write!(f, "unable to detect KDT version"),
            Self::InvalidArgument(msg) => write!(f, "invalid argument: {msg}"),
            Self::Spu(err) => write!(f, "SPU error: {err}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Spu(err) => Some(err),
            _ => None,
        }
    }
}

impl From<libspu2::Error> for Error {
    fn from(err: libspu2::Error) -> Self {
        Self::Spu(err)
    }
}

const _: () = {
    const fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<Error>();
    assert_send_sync::<KdtVersion>();
    assert_send_sync::<KdtSequenceView<'static>>();
    assert_send_sync::<KdtTrackView<'static>>();
    assert_send_sync::<KdtCommand>();
    assert_send_sync::<KdtEvent>();
    assert_send_sync::<VabBankView<'static>>();
    assert_send_sync::<VabHeader>();
    assert_send_sync::<ProgramAttr>();
    assert_send_sync::<ToneAttr>();
    assert_send_sync::<ReverbPreset>();
    assert_send_sync::<ReverbParams>();
    #[cfg(feature = "alloc")]
    {
        assert_send_sync::<KdtSequence>();
        assert_send_sync::<VabBank>();
        assert_send_sync::<VabUpload>();
        assert_send_sync::<RenderConfig>();
        assert_send_sync::<RenderResult>();
        assert_send_sync::<ChannelSnapshot>();
        assert_send_sync::<KdtPlayer>();
    }
};
