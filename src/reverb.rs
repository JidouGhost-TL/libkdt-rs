// SPDX-License-Identifier: MIT OR Apache-2.0

//! PlayStation 1 SPU hardware reverb presets and register parameter blocks.

/// The 10 standard PlayStation 1 SPU hardware reverb presets (`0..=9`).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ReverbPreset {
    /// Reverb disabled (`0` bytes work area).
    #[default]
    Off,
    /// Small room (`0x26C0` bytes work area).
    Room,
    /// Small studio (`0x1F40` bytes work area).
    StudioA,
    /// Medium studio (`0x4840` bytes work area).
    StudioB,
    /// Large studio (`0x6FE0` bytes work area).
    StudioC,
    /// Concert hall (`0xADE0` bytes work area).
    Hall,
    /// Large space (`0xF6C0` bytes work area).
    Space,
    /// Stereo echo (`0x18040` bytes work area).
    Echo,
    /// Stereo delay (`0x18040` bytes work area).
    Delay,
    /// Half-echo / pipe (`0x3C00` bytes work area).
    Pipe,
}

impl ReverbPreset {
    /// All 10 standard presets in canonical hardware index order (`0..=9`).
    pub const ALL: [Self; 10] = [
        Self::Off,
        Self::Room,
        Self::StudioA,
        Self::StudioB,
        Self::StudioC,
        Self::Hall,
        Self::Space,
        Self::Echo,
        Self::Delay,
        Self::Pipe,
    ];

    /// Converts a standard hardware preset index (`0..=9`) into a [`ReverbPreset`].
    #[inline]
    #[must_use]
    pub const fn from_index(index: u8) -> Option<Self> {
        match index {
            0 => Some(Self::Off),
            1 => Some(Self::Room),
            2 => Some(Self::StudioA),
            3 => Some(Self::StudioB),
            4 => Some(Self::StudioC),
            5 => Some(Self::Hall),
            6 => Some(Self::Space),
            7 => Some(Self::Echo),
            8 => Some(Self::Delay),
            9 => Some(Self::Pipe),
            _ => None,
        }
    }

    /// Required PS1 SPU RAM work-area size in bytes for this preset.
    #[inline]
    #[must_use]
    pub const fn work_area_size(self) -> u32 {
        match self {
            Self::Off => 0x0000,
            Self::Room => 0x26C0,
            Self::StudioA => 0x1F40,
            Self::StudioB => 0x4840,
            Self::StudioC => 0x6FE0,
            Self::Hall => 0xADE0,
            Self::Space => 0xF6C0,
            Self::Echo => 0x18040,
            Self::Delay => 0x18040,
            Self::Pipe => 0x3C00,
        }
    }

    /// Returns the exact 32 hardware reverb register parameters for this preset.
    #[must_use]
    pub const fn params(self) -> ReverbParams {
        match self {
            Self::Off => ReverbParams::ZERO,
            Self::Room => ReverbParams {
                d_apf1: 0x007D,
                d_apf2: 0x005B,
                v_iir: 0x6D80_u16 as i16,
                v_comb1: 0x54B8_u16 as i16,
                v_comb2: 0xBED0_u16 as i16,
                v_comb3: 0x0000,
                v_comb4: 0x0000,
                v_wall: 0xBA80_u16 as i16,
                v_apf1: 0x5800_u16 as i16,
                v_apf2: 0x5300_u16 as i16,
                m_lsame: 0x04D6,
                m_rsame: 0x0333,
                m_lcomb1: 0x03F0,
                m_rcomb1: 0x0227,
                m_lcomb2: 0x0374,
                m_rcomb2: 0x01EF,
                d_lsame: 0x0334,
                d_rsame: 0x01B5,
                m_ldiff: 0x0000,
                m_rdiff: 0x0000,
                m_lcomb3: 0x0000,
                m_rcomb3: 0x0000,
                m_lcomb4: 0x0000,
                m_rcomb4: 0x0000,
                d_ldiff: 0x0000,
                d_rdiff: 0x0000,
                m_lapf1: 0x01B4,
                m_rapf1: 0x0136,
                m_lapf2: 0x00B8,
                m_rapf2: 0x005C,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
            Self::StudioA => ReverbParams {
                d_apf1: 0x0033,
                d_apf2: 0x0025,
                v_iir: 0x70F0_u16 as i16,
                v_comb1: 0x4FA8_u16 as i16,
                v_comb2: 0xBCE0_u16 as i16,
                v_comb3: 0x4410_u16 as i16,
                v_comb4: 0xC0F0_u16 as i16,
                v_wall: 0x9C00_u16 as i16,
                v_apf1: 0x5280_u16 as i16,
                v_apf2: 0x4EC0_u16 as i16,
                m_lsame: 0x03E4,
                m_rsame: 0x031B,
                m_lcomb1: 0x03A4,
                m_rcomb1: 0x02AF,
                m_lcomb2: 0x0372,
                m_rcomb2: 0x0266,
                d_lsame: 0x031C,
                d_rsame: 0x025D,
                m_ldiff: 0x025C,
                m_rdiff: 0x018E,
                m_lcomb3: 0x022F,
                m_rcomb3: 0x0135,
                m_lcomb4: 0x01D2,
                m_rcomb4: 0x00B7,
                d_ldiff: 0x018F,
                d_rdiff: 0x00B5,
                m_lapf1: 0x00B4,
                m_rapf1: 0x0080,
                m_lapf2: 0x004C,
                m_rapf2: 0x0026,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
            Self::StudioB => ReverbParams {
                d_apf1: 0x00B1,
                d_apf2: 0x007F,
                v_iir: 0x70F0_u16 as i16,
                v_comb1: 0x4FA8_u16 as i16,
                v_comb2: 0xBCE0_u16 as i16,
                v_comb3: 0x4510_u16 as i16,
                v_comb4: 0xBEF0_u16 as i16,
                v_wall: 0xB4C0_u16 as i16,
                v_apf1: 0x5280_u16 as i16,
                v_apf2: 0x4EC0_u16 as i16,
                m_lsame: 0x0904,
                m_rsame: 0x076B,
                m_lcomb1: 0x0824,
                m_rcomb1: 0x065F,
                m_lcomb2: 0x07A2,
                m_rcomb2: 0x0616,
                d_lsame: 0x076C,
                d_rsame: 0x05ED,
                m_ldiff: 0x05EC,
                m_rdiff: 0x042E,
                m_lcomb3: 0x050F,
                m_rcomb3: 0x0305,
                m_lcomb4: 0x0462,
                m_rcomb4: 0x02B7,
                d_ldiff: 0x042F,
                d_rdiff: 0x0265,
                m_lapf1: 0x0264,
                m_rapf1: 0x01B2,
                m_lapf2: 0x0100,
                m_rapf2: 0x0080,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
            Self::StudioC => ReverbParams {
                d_apf1: 0x00E3,
                d_apf2: 0x00A9,
                v_iir: 0x6F60_u16 as i16,
                v_comb1: 0x4FA8_u16 as i16,
                v_comb2: 0xBCE0_u16 as i16,
                v_comb3: 0x4510_u16 as i16,
                v_comb4: 0xBEF0_u16 as i16,
                v_wall: 0xA680_u16 as i16,
                v_apf1: 0x5680_u16 as i16,
                v_apf2: 0x52C0_u16 as i16,
                m_lsame: 0x0DFB,
                m_rsame: 0x0B58,
                m_lcomb1: 0x0D09,
                m_rcomb1: 0x0A3C,
                m_lcomb2: 0x0BD9,
                m_rcomb2: 0x0973,
                d_lsame: 0x0B59,
                d_rsame: 0x08DA,
                m_ldiff: 0x08D9,
                m_rdiff: 0x05E9,
                m_lcomb3: 0x07EC,
                m_rcomb3: 0x04B0,
                m_lcomb4: 0x06EF,
                m_rcomb4: 0x03D2,
                d_ldiff: 0x05EA,
                d_rdiff: 0x031D,
                m_lapf1: 0x031C,
                m_rapf1: 0x0238,
                m_lapf2: 0x0154,
                m_rapf2: 0x00AA,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
            Self::Hall => ReverbParams {
                d_apf1: 0x01A5,
                d_apf2: 0x0139,
                v_iir: 0x6000_u16 as i16,
                v_comb1: 0x5000_u16 as i16,
                v_comb2: 0x4C00_u16 as i16,
                v_comb3: 0xB800_u16 as i16,
                v_comb4: 0xBC00_u16 as i16,
                v_wall: 0xC000_u16 as i16,
                v_apf1: 0x6000_u16 as i16,
                v_apf2: 0x5C00_u16 as i16,
                m_lsame: 0x15BA,
                m_rsame: 0x11BB,
                m_lcomb1: 0x14C2,
                m_rcomb1: 0x10BD,
                m_lcomb2: 0x11BC,
                m_rcomb2: 0x0DC1,
                d_lsame: 0x11C0,
                d_rsame: 0x0DC3,
                m_ldiff: 0x0DC0,
                m_rdiff: 0x09C1,
                m_lcomb3: 0x0BC4,
                m_rcomb3: 0x07C1,
                m_lcomb4: 0x0A00,
                m_rcomb4: 0x06CD,
                d_ldiff: 0x09C2,
                d_rdiff: 0x05C1,
                m_lapf1: 0x05C0,
                m_rapf1: 0x041A,
                m_lapf2: 0x0274,
                m_rapf2: 0x013A,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
            Self::Space => ReverbParams {
                d_apf1: 0x033D,
                d_apf2: 0x0231,
                v_iir: 0x7E00_u16 as i16,
                v_comb1: 0x5000_u16 as i16,
                v_comb2: 0xB400_u16 as i16,
                v_comb3: 0xB000_u16 as i16,
                v_comb4: 0x4C00_u16 as i16,
                v_wall: 0xB000_u16 as i16,
                v_apf1: 0x6000_u16 as i16,
                v_apf2: 0x5400_u16 as i16,
                m_lsame: 0x1ED6,
                m_rsame: 0x1A31,
                m_lcomb1: 0x1D14,
                m_rcomb1: 0x183B,
                m_lcomb2: 0x1BC2,
                m_rcomb2: 0x16B2,
                d_lsame: 0x1A32,
                d_rsame: 0x15EF,
                m_ldiff: 0x15EE,
                m_rdiff: 0x1055,
                m_lcomb3: 0x1334,
                m_rcomb3: 0x0F2D,
                m_lcomb4: 0x11F6,
                m_rcomb4: 0x0C5D,
                d_ldiff: 0x1056,
                d_rdiff: 0x0AE1,
                m_lapf1: 0x0AE0,
                m_rapf1: 0x07A2,
                m_lapf2: 0x0464,
                m_rapf2: 0x0232,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
            Self::Echo => ReverbParams {
                d_apf1: 0x0001,
                d_apf2: 0x0001,
                v_iir: 0x7FFF_u16 as i16,
                v_comb1: 0x7FFF_u16 as i16,
                v_comb2: 0x0000,
                v_comb3: 0x0000,
                v_comb4: 0x0000,
                v_wall: 0x8100_u16 as i16,
                v_apf1: 0x0000,
                v_apf2: 0x0000,
                m_lsame: 0x1FFF,
                m_rsame: 0x0FFF,
                m_lcomb1: 0x1005,
                m_rcomb1: 0x0005,
                m_lcomb2: 0x0000,
                m_rcomb2: 0x0000,
                d_lsame: 0x1005,
                d_rsame: 0x0005,
                m_ldiff: 0x0000,
                m_rdiff: 0x0000,
                m_lcomb3: 0x0000,
                m_rcomb3: 0x0000,
                m_lcomb4: 0x0000,
                m_rcomb4: 0x0000,
                d_ldiff: 0x0000,
                d_rdiff: 0x0000,
                m_lapf1: 0x1004,
                m_rapf1: 0x1002,
                m_lapf2: 0x0004,
                m_rapf2: 0x0002,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
            Self::Delay => ReverbParams {
                d_apf1: 0x0001,
                d_apf2: 0x0001,
                v_iir: 0x7FFF_u16 as i16,
                v_comb1: 0x7FFF_u16 as i16,
                v_comb2: 0x0000,
                v_comb3: 0x0000,
                v_comb4: 0x0000,
                v_wall: 0x0000,
                v_apf1: 0x0000,
                v_apf2: 0x0000,
                m_lsame: 0x1FFF,
                m_rsame: 0x0FFF,
                m_lcomb1: 0x1005,
                m_rcomb1: 0x0005,
                m_lcomb2: 0x0000,
                m_rcomb2: 0x0000,
                d_lsame: 0x1005,
                d_rsame: 0x0005,
                m_ldiff: 0x0000,
                m_rdiff: 0x0000,
                m_lcomb3: 0x0000,
                m_rcomb3: 0x0000,
                m_lcomb4: 0x0000,
                m_rcomb4: 0x0000,
                d_ldiff: 0x0000,
                d_rdiff: 0x0000,
                m_lapf1: 0x1004,
                m_rapf1: 0x1002,
                m_lapf2: 0x0004,
                m_rapf2: 0x0002,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
            Self::Pipe => ReverbParams {
                d_apf1: 0x0017,
                d_apf2: 0x0013,
                v_iir: 0x70F0_u16 as i16,
                v_comb1: 0x4FA8_u16 as i16,
                v_comb2: 0xBCE0_u16 as i16,
                v_comb3: 0x4510_u16 as i16,
                v_comb4: 0xBEF0_u16 as i16,
                v_wall: 0x8500_u16 as i16,
                v_apf1: 0x5F80_u16 as i16,
                v_apf2: 0x54C0_u16 as i16,
                m_lsame: 0x0371,
                m_rsame: 0x02AF,
                m_lcomb1: 0x02E5,
                m_rcomb1: 0x01DF,
                m_lcomb2: 0x02B0,
                m_rcomb2: 0x01D7,
                d_lsame: 0x0358,
                d_rsame: 0x026A,
                m_ldiff: 0x01D6,
                m_rdiff: 0x011E,
                m_lcomb3: 0x012D,
                m_rcomb3: 0x00B1,
                m_lcomb4: 0x011F,
                m_rcomb4: 0x0059,
                d_ldiff: 0x01A0,
                d_rdiff: 0x00E3,
                m_lapf1: 0x0058,
                m_rapf1: 0x0040,
                m_lapf2: 0x0028,
                m_rapf2: 0x0014,
                v_lin: 0x8000_u16 as i16,
                v_rin: 0x8000_u16 as i16,
            },
        }
    }
}

/// All 32 PlayStation 1 SPU hardware reverb register parameters.
///
/// Address/offset fields (`d_*`, `m_*`) are stored in standard PS1 8-byte
/// units (`byte_offset = reg << 3`). Coefficient fields (`v_*`) are signed
/// 16-bit Q1.15 fixed-point gains.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ReverbParams {
    /// Reverb APF offset 1 (`dAPF1`).
    pub d_apf1: u16,
    /// Reverb APF offset 2 (`dAPF2`).
    pub d_apf2: u16,
    /// Reflection volume 1 (`vIIR`).
    pub v_iir: i16,
    /// Comb volume 1 (`vCOMB1`).
    pub v_comb1: i16,
    /// Comb volume 2 (`vCOMB2`).
    pub v_comb2: i16,
    /// Comb volume 3 (`vCOMB3`).
    pub v_comb3: i16,
    /// Comb volume 4 (`vCOMB4`).
    pub v_comb4: i16,
    /// Reflection volume 2 (`vWALL`).
    pub v_wall: i16,
    /// APF volume 1 (`vAPF1`).
    pub v_apf1: i16,
    /// APF volume 2 (`vAPF2`).
    pub v_apf2: i16,
    /// Same-side reflection address 1 left (`mLSAME`).
    pub m_lsame: u16,
    /// Same-side reflection address 1 right (`mRSAME`).
    pub m_rsame: u16,
    /// Comb address 1 left (`mLCOMB1`).
    pub m_lcomb1: u16,
    /// Comb address 1 right (`mRCOMB1`).
    pub m_rcomb1: u16,
    /// Comb address 2 left (`mLCOMB2`).
    pub m_lcomb2: u16,
    /// Comb address 2 right (`mRCOMB2`).
    pub m_rcomb2: u16,
    /// Same-side reflection address 2 left (`dLSAME`).
    pub d_lsame: u16,
    /// Same-side reflection address 2 right (`dRSAME`).
    pub d_rsame: u16,
    /// Different-side reflection address 1 left (`mLDIFF`).
    pub m_ldiff: u16,
    /// Different-side reflection address 1 right (`mRDIFF`).
    pub m_rdiff: u16,
    /// Comb address 3 left (`mLCOMB3`).
    pub m_lcomb3: u16,
    /// Comb address 3 right (`mRCOMB3`).
    pub m_rcomb3: u16,
    /// Comb address 4 left (`mLCOMB4`).
    pub m_lcomb4: u16,
    /// Comb address 4 right (`mRCOMB4`).
    pub m_rcomb4: u16,
    /// Different-side reflection address 2 left (`dLDIFF`).
    pub d_ldiff: u16,
    /// Different-side reflection address 2 right (`dRDIFF`).
    pub d_rdiff: u16,
    /// APF address 1 left (`mLAPF1`).
    pub m_lapf1: u16,
    /// APF address 1 right (`mRAPF1`).
    pub m_rapf1: u16,
    /// APF address 2 left (`mLAPF2`).
    pub m_lapf2: u16,
    /// APF address 2 right (`mRAPF2`).
    pub m_rapf2: u16,
    /// Input volume left (`vLIN`).
    pub v_lin: i16,
    /// Input volume right (`vRIN`).
    pub v_rin: i16,
}

impl ReverbParams {
    /// All-zero reverb parameter block (used for [`ReverbPreset::Off`]).
    pub const ZERO: Self = Self {
        d_apf1: 0,
        d_apf2: 0,
        v_iir: 0,
        v_comb1: 0,
        v_comb2: 0,
        v_comb3: 0,
        v_comb4: 0,
        v_wall: 0,
        v_apf1: 0,
        v_apf2: 0,
        m_lsame: 0,
        m_rsame: 0,
        m_lcomb1: 0,
        m_rcomb1: 0,
        m_lcomb2: 0,
        m_rcomb2: 0,
        d_lsame: 0,
        d_rsame: 0,
        m_ldiff: 0,
        m_rdiff: 0,
        m_lcomb3: 0,
        m_rcomb3: 0,
        m_lcomb4: 0,
        m_rcomb4: 0,
        d_ldiff: 0,
        d_rdiff: 0,
        m_lapf1: 0,
        m_rapf1: 0,
        m_lapf2: 0,
        m_rapf2: 0,
        v_lin: 0,
        v_rin: 0,
    };

    /// Returns all 32 register words in exact PS1 SPU `RVB_BASE..RVB_END` order
    /// (`0x1F801DC0..=0x1F801DFE`).
    #[inline]
    #[must_use]
    pub const fn to_ps1_registers(&self) -> [u16; 32] {
        [
            self.d_apf1,
            self.d_apf2,
            self.v_iir as u16,
            self.v_comb1 as u16,
            self.v_comb2 as u16,
            self.v_comb3 as u16,
            self.v_comb4 as u16,
            self.v_wall as u16,
            self.v_apf1 as u16,
            self.v_apf2 as u16,
            self.m_lsame,
            self.m_rsame,
            self.m_lcomb1,
            self.m_rcomb1,
            self.m_lcomb2,
            self.m_rcomb2,
            self.d_lsame,
            self.d_rsame,
            self.m_ldiff,
            self.m_rdiff,
            self.m_lcomb3,
            self.m_rcomb3,
            self.m_lcomb4,
            self.m_rcomb4,
            self.d_ldiff,
            self.d_rdiff,
            self.m_lapf1,
            self.m_rapf1,
            self.m_lapf2,
            self.m_rapf2,
            self.v_lin as u16,
            self.v_rin as u16,
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_std_reverb_preset_ordering_and_registers() {
        for (idx, &preset) in ReverbPreset::ALL.iter().enumerate() {
            assert_eq!(ReverbPreset::from_index(idx as u8), Some(preset));
            if idx > 0 {
                assert!(ReverbPreset::ALL[idx - 1] < preset);
                assert!(preset.work_area_size() > 0);
            }
        }
        assert_eq!(ReverbPreset::Off.work_area_size(), 0);
        assert_eq!(ReverbPreset::Off.params().to_ps1_registers(), [0u16; 32]);
        assert_eq!(ReverbPreset::Hall.params().to_ps1_registers()[0], 0x01A5);
        assert_eq!(ReverbPreset::from_index(10), None);
    }
}
