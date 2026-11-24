// SPDX-License-Identifier: MIT OR Apache-2.0

//! Minimal standalone Sony VAB (`.VH` + `.VB`) sample bank loader.
//!
//! [`VabBankView`] provides zero-allocation borrowed parsing usable in
//! `#![no_std]` environments without `alloc`. With the `alloc` feature
//! enabled, [`VabBank`] provides an owned representation with SPU upload, so
//! `libkdt` is fully self-contained without any companion bank crate.
//!
//! Layout (little-endian throughout): 32-byte header, fixed program table
//! (128 × 16 B for v5+, 64 × 16 B for v1–v4), compact tone table
//! (`program_count` × 16 × 32 B), 256-entry `u16` VAG size table (size × 8
//! bytes for v5+, × 4 for v1–v4), then concatenated SPU-ADPCM bodies. Split
//! banks carry tables in `.VH` and bodies in `.VB`.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

use crate::{Error, Result};

/// On-disk VAB magic word (`"pBAV"` byte order, `VABp` read big-endian).
pub const VAB_MAGIC: [u8; 4] = *b"pBAV";

/// VAB header size in bytes (`32`).
pub const VAB_HEADER_BYTES: usize = 32;

/// Default PS1 SPU RAM base for uploaded VAB bodies (`0x1010`, past the
/// hardware-reserved low region).
pub const VAB_SPU_BASE: u32 = 0x1010;

/// Parsed 32-byte VAB header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VabHeader {
    /// Raw little-endian magic word at `+0x00` (`0x56414270` for `"pBAV"`).
    pub magic: u32,
    /// Format version at `+0x04`.
    pub version: u32,
    /// Bank identifier at `+0x08`.
    pub id: u32,
    /// Declared total file size at `+0x0C`.
    pub file_size: u32,
    /// Reserved word at `+0x10`.
    pub reserved0: u16,
    /// Populated program count at `+0x12`.
    pub program_count: u16,
    /// Tone count at `+0x14`.
    pub tone_count: u16,
    /// Populated VAG count at `+0x16`.
    pub vag_count: u16,
    /// Bank master volume (`0..=127`) at `+0x18`.
    pub master_volume: u8,
    /// Bank master pan (`0..=127`, `64` = center) at `+0x19`.
    pub master_pan: u8,
    /// Bank attribute 1 at `+0x1A`.
    pub attr1: u8,
    /// Bank attribute 2 at `+0x1B`.
    pub attr2: u8,
    /// Reserved word at `+0x1C`.
    pub reserved1: u32,
}

impl VabHeader {
    /// Returns `true` for the v5+ layout (128-entry program table, 8-byte
    /// VAG size units).
    #[inline]
    #[must_use]
    pub const fn is_v5_plus(&self) -> bool {
        (self.magic & 0xFF) == 0x70 && self.version > 4
    }

    /// Number of fixed program-table entries (`128` v5+, `64` v1–v4).
    #[inline]
    #[must_use]
    pub const fn program_table_entries(&self) -> usize {
        if self.is_v5_plus() {
            128
        } else {
            64
        }
    }

    /// Byte multiplier for each VAG size-table entry (`8` v5+, `4` v1–v4).
    #[inline]
    #[must_use]
    pub const fn vag_unit_bytes(&self) -> usize {
        if self.is_v5_plus() {
            8
        } else {
            4
        }
    }
}

/// Typed view of a 16-byte VAB Program (`ProgAtr`) record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProgramAttr {
    /// Number of valid tones in this program (`0..=16`).
    pub tones: u8,
    /// Program master volume (`0..=127`).
    pub master_volume: u8,
    /// Program priority (voice-stealing hint).
    pub priority: u8,
    /// Program play mode.
    pub mode: u8,
    /// Program pan (`0..=127`, `64` = center).
    pub master_pan: u8,
}

impl ProgramAttr {
    /// Decodes a `ProgramAttr` from a raw 16-byte record.
    #[inline]
    #[must_use]
    pub const fn from_bytes(b: &[u8; 16]) -> Self {
        Self {
            tones: b[0],
            master_volume: b[1],
            priority: b[2],
            mode: b[3],
            master_pan: b[4],
        }
    }
}

/// Typed view of a 32-byte VAB Tone (`VagAtr`) record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ToneAttr {
    /// Voice priority (voice-stealing hint).
    pub priority: u8,
    /// Play mode (bit `0x04` = SPU hardware reverb send).
    pub mode: u8,
    /// Tone volume (`0..=127`).
    pub volume: u8,
    /// Tone pan (`0..=127`, `64` = center).
    pub pan: u8,
    /// Center note at unity playback ratio.
    pub center: u8,
    /// Center-note fine tune in 1/128 semitone.
    pub shift: u8,
    /// Key-range low bound (inclusive).
    pub min: u8,
    /// Key-range high bound (inclusive).
    pub max: u8,
    /// Packed ADSR low half.
    pub adsr1: u16,
    /// Packed ADSR high half.
    pub adsr2: u16,
    /// Parent program index.
    pub program: i16,
    /// One-based VAG slot reference (`0` = no sample).
    pub vag: i16,
}

impl ToneAttr {
    /// Decodes a `ToneAttr` from a raw 32-byte record.
    #[inline]
    #[must_use]
    pub const fn from_bytes(b: &[u8; 32]) -> Self {
        Self {
            priority: b[0],
            mode: b[1],
            volume: b[2],
            pan: b[3],
            center: b[4],
            shift: b[5],
            min: b[6],
            max: b[7],
            adsr1: u16::from_le_bytes([b[16], b[17]]),
            adsr2: u16::from_le_bytes([b[18], b[19]]),
            program: i16::from_le_bytes([b[20], b[21]]),
            vag: i16::from_le_bytes([b[22], b[23]]),
        }
    }
}

fn parse_header(bytes: &[u8]) -> Result<VabHeader> {
    if bytes.len() < VAB_HEADER_BYTES {
        return Err(Error::UnexpectedEof("VAB header"));
    }
    let magic = read_u32_le(bytes, 0);
    // Upper 24 bits must spell "VAB"; v5+ additionally requires exact "pBAV".
    if (magic >> 8) != 0x0056_4142 {
        return Err(Error::InvalidMagic);
    }
    let version = read_u32_le(bytes, 4);
    if version > 4 && magic != u32::from_le_bytes(VAB_MAGIC) {
        return Err(Error::InvalidMagic);
    }
    Ok(VabHeader {
        magic,
        version,
        id: read_u32_le(bytes, 8),
        file_size: read_u32_le(bytes, 12),
        reserved0: read_u16_le(bytes, 16),
        program_count: read_u16_le(bytes, 18),
        tone_count: read_u16_le(bytes, 20),
        vag_count: read_u16_le(bytes, 22),
        master_volume: bytes[24],
        master_pan: bytes[25],
        attr1: bytes[26],
        attr2: bytes[27],
        reserved1: read_u32_le(bytes, 28),
    })
}

fn table_bytes(header: &VabHeader) -> Result<usize> {
    let entries = header.program_table_entries();
    if (header.program_count as usize) > entries {
        return Err(Error::InvalidHeader(
            "program_count exceeds fixed program table",
        ));
    }
    VAB_HEADER_BYTES
        .checked_add(entries * 16)
        .and_then(|n| {
            (header.program_count as usize)
                .checked_mul(16 * 32)
                .and_then(|t| n.checked_add(t))
        })
        .and_then(|n| n.checked_add(256 * 2))
        .ok_or(Error::OutOfRange("VAB table extent overflow"))
}

/// Zero-allocation borrowed view of a parsed VAB bank.
///
/// Usable in pure `#![no_std]` builds without the `alloc` feature.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VabBankView<'a> {
    /// Parsed bank header.
    pub header: VabHeader,
    programs_raw: &'a [u8],
    tones_raw: &'a [u8],
    vag_sizes_raw: &'a [u8],
    bodies: &'a [u8],
}

impl<'a> VabBankView<'a> {
    /// Parses a combined VAB bank (tables + bodies in one buffer).
    pub fn parse_vab(bytes: &'a [u8]) -> Result<Self> {
        let header = parse_header(bytes)?;
        let tables = table_bytes(&header)?;
        if bytes.len() < tables {
            return Err(Error::UnexpectedEof("VAB tables"));
        }
        Self::from_parts(header, bytes, tables, &bytes[tables..])
    }

    /// Parses a split bank: header/tables from `vh`, bodies from `vb`.
    pub fn parse_vh_vb(vh: &'a [u8], vb: &'a [u8]) -> Result<Self> {
        let header = parse_header(vh)?;
        let tables = table_bytes(&header)?;
        if vh.len() < tables {
            return Err(Error::UnexpectedEof("VAB .VH tables"));
        }
        Self::from_parts(header, vh, tables, vb)
    }

    fn from_parts(
        header: VabHeader,
        tables_src: &'a [u8],
        tables: usize,
        bodies: &'a [u8],
    ) -> Result<Self> {
        let entries = header.program_table_entries();
        let prog_end = VAB_HEADER_BYTES + entries * 16;
        let tone_bytes = (header.program_count as usize) * 16 * 32;
        let view = Self {
            header,
            programs_raw: &tables_src[VAB_HEADER_BYTES..prog_end],
            tones_raw: &tables_src[prog_end..prog_end + tone_bytes],
            vag_sizes_raw: &tables_src[prog_end + tone_bytes..tables],
            bodies,
        };
        if bodies.len() < view.expected_body_bytes() {
            return Err(Error::UnexpectedEof("VAB bodies"));
        }
        Ok(view)
    }

    /// Total declared body bytes (sum of VAG sizes × unit).
    #[must_use]
    pub fn expected_body_bytes(&self) -> usize {
        let unit = self.header.vag_unit_bytes();
        (0..256).map(|i| self.vag_size(i) as usize * unit).sum()
    }

    /// Returns the raw ADPCM bodies slice.
    #[inline]
    #[must_use]
    pub const fn bodies(&self) -> &'a [u8] {
        self.bodies
    }

    /// Decodes the program-table entry at `slot`.
    #[must_use]
    pub fn program(&self, slot: usize) -> Option<ProgramAttr> {
        let raw: &[u8; 16] = self
            .programs_raw
            .get(slot * 16..slot * 16 + 16)?
            .try_into()
            .ok()?;
        Some(ProgramAttr::from_bytes(raw))
    }

    /// Number of compact tone blocks (`program_count`).
    #[inline]
    #[must_use]
    pub const fn tone_block_count(&self) -> usize {
        self.header.program_count as usize
    }

    /// Decodes tone `tone` (`0..16`) of compact block `block`.
    #[must_use]
    pub fn tone(&self, block: usize, tone: usize) -> Option<ToneAttr> {
        if tone >= 16 {
            return None;
        }
        let idx = block * 16 + tone;
        let raw: &[u8; 32] = self
            .tones_raw
            .get(idx * 32..idx * 32 + 32)?
            .try_into()
            .ok()?;
        Some(ToneAttr::from_bytes(raw))
    }

    /// Finds the compact block serving program-table `slot`, if populated.
    #[must_use]
    pub fn block_for_program(&self, slot: usize) -> Option<usize> {
        for block in 0..self.tone_block_count() {
            if let Some(first) = self.tone(block, 0) {
                if first.program >= 0 && first.program as usize == slot {
                    return Some(block);
                }
            }
        }
        // Fallback: identity mapping when tone program fields are unset.
        if slot < self.tone_block_count() {
            return Some(slot);
        }
        None
    }

    /// Returns the size-table value for VAG `slot_id` (`0..=255`).
    #[must_use]
    pub fn vag_size(&self, slot_id: usize) -> u16 {
        if slot_id >= 256 {
            return 0;
        }
        read_u16_le(self.vag_sizes_raw, slot_id * 2)
    }

    /// Returns the ADPCM body for one-based VAG `slot_id`, if populated.
    #[must_use]
    pub fn vag_body(&self, slot_id: usize) -> Option<&'a [u8]> {
        if slot_id == 0 || slot_id >= 256 || self.vag_size(slot_id) == 0 {
            return None;
        }
        let unit = self.header.vag_unit_bytes();
        let mut off = 0usize;
        for i in 1..slot_id {
            off += self.vag_size(i) as usize * unit;
        }
        let len = self.vag_size(slot_id) as usize * unit;
        self.bodies.get(off..off + len)
    }

    /// Converts this borrowed view into an owned [`VabBank`].
    #[cfg(feature = "alloc")]
    #[must_use]
    pub fn to_owned_bank(&self) -> VabBank {
        let entries = self.header.program_table_entries();
        let mut programs = Vec::with_capacity(entries);
        for slot in 0..entries {
            if let Some(p) = self.program(slot) {
                programs.push(p);
            }
        }
        let mut tones = Vec::with_capacity(self.tone_block_count() * 16);
        for block in 0..self.tone_block_count() {
            for t in 0..16 {
                if let Some(tone) = self.tone(block, t) {
                    tones.push(tone);
                }
            }
        }
        let mut vag_sizes = Vec::with_capacity(256);
        for i in 0..256 {
            vag_sizes.push(self.vag_size(i));
        }
        VabBank {
            header: self.header,
            programs,
            tones,
            vag_sizes,
            bodies: self.bodies.to_vec(),
        }
    }
}

/// Result of uploading a [`VabBank`] into SPU RAM.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VabUpload {
    /// SPU RAM base address at which bodies were uploaded.
    pub base_addr: u32,
    /// Resolved SPU RAM start address per VAG slot (`None` if empty).
    pub vag_addrs: [Option<u32>; 256],
}

/// Owned parsed Sony VAB sample bank.
#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VabBank {
    /// Parsed bank header.
    pub header: VabHeader,
    /// Fixed program table (`128` entries v5+, `64` v1–v4).
    pub programs: Vec<ProgramAttr>,
    /// Compact tone table (`program_count * 16` entries).
    pub tones: Vec<ToneAttr>,
    /// 256-entry VAG size table (units per [`VabHeader::vag_unit_bytes`]).
    pub vag_sizes: Vec<u16>,
    /// Concatenated SPU-ADPCM bodies.
    pub bodies: Vec<u8>,
}

#[cfg(feature = "alloc")]
impl VabBank {
    /// Parses a combined VAB bank (tables + bodies in one buffer).
    pub fn parse_vab(bytes: &[u8]) -> Result<Self> {
        Ok(VabBankView::parse_vab(bytes)?.to_owned_bank())
    }

    /// Parses a split bank: header/tables from `vh`, bodies from `vb`.
    pub fn parse_vh_vb(vh: &[u8], vb: &[u8]) -> Result<Self> {
        Ok(VabBankView::parse_vh_vb(vh, vb)?.to_owned_bank())
    }

    /// Returns the ADPCM body for one-based VAG `slot_id`, if populated.
    #[must_use]
    pub fn vag_body(&self, slot_id: usize) -> Option<&[u8]> {
        if slot_id == 0 || slot_id >= 256 {
            return None;
        }
        let size = *self.vag_sizes.get(slot_id)?;
        if size == 0 {
            return None;
        }
        let unit = self.header.vag_unit_bytes();
        let mut off = 0usize;
        for i in 1..slot_id {
            off += (*self.vag_sizes.get(i)? as usize) * unit;
        }
        self.bodies.get(off..off + (size as usize) * unit)
    }

    /// Returns `(program, tone block)` for program-table `slot`, if populated.
    #[must_use]
    pub fn program_block(&self, slot: usize) -> Option<(ProgramAttr, &[ToneAttr])> {
        let program = *self.programs.get(slot)?;
        let blocks = self.tones.len() / 16;
        for block in 0..blocks {
            let first = self.tones[block * 16];
            if first.program >= 0 && first.program as usize == slot {
                return Some((program, &self.tones[block * 16..block * 16 + 16]));
            }
        }
        if slot < blocks {
            return Some((program, &self.tones[slot * 16..slot * 16 + 16]));
        }
        None
    }

    /// Uploads all bodies into `device` SPU RAM at [`VAB_SPU_BASE`].
    pub fn upload_to_spu(&self, device: &mut libspu2::Device) -> Result<VabUpload> {
        let end = (VAB_SPU_BASE as usize)
            .checked_add(self.bodies.len())
            .ok_or(Error::OutOfRange("VAB upload range overflow"))?;
        if end > device.ram_size() {
            return Err(Error::OutOfRange("VAB bodies exceed SPU RAM size"));
        }
        device.write_ram(VAB_SPU_BASE, &self.bodies)?;
        let unit = self.header.vag_unit_bytes();
        let mut vag_addrs = [None; 256];
        let mut off = 0u32;
        for (slot, addr) in vag_addrs.iter_mut().enumerate().skip(1) {
            let size = self.vag_sizes.get(slot).copied().unwrap_or(0) as u32;
            if size > 0 {
                *addr = Some(VAB_SPU_BASE + off);
            }
            off += size * (unit as u32);
        }
        Ok(VabUpload {
            base_addr: VAB_SPU_BASE,
            vag_addrs,
        })
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
