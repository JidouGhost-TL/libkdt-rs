// SPDX-License-Identifier: MIT OR Apache-2.0

//! Konami KDT sequencer playback engine driving `libspu2::Device::new_ps1()`.

use alloc::vec::Vec;
use core::fmt;

use libspu2::registers::{ps1, CONTROL_ENABLE, CONTROL_REVERB, CONTROL_UNMUTE};
use libspu2::{Device, StereoI16, VoiceState, PS1_RAM_SIZE, PS1_SAMPLE_RATE, VOICES_PER_CORE};

use crate::bank::{ToneAttr, VabBank, VabUpload};
use crate::reverb::{ReverbParams, ReverbPreset};
use crate::sequence::{KdtCommand, KdtSequence};
use crate::version::{KDT_RC2_CLOCK_HZ, KDT_RC2_DIVIDER};
use crate::{Error, Result};

/// Number of logical MIDI-style channels (`16`).
pub const MAX_CHANNELS: usize = 16;

/// Default per-track tempo accumulator increment (`val = 63 => 128`).
const DEFAULT_TEMPO: u16 = 128;

/// VAB `ToneAttr::mode` bit indicating SPU hardware reverb send.
const TONE_MODE_REVERB: u8 = 0x04;

/// 12-TET pitch table for octave 4 (`C4 = 0x1000` unity PS1 SPU pitch).
const PITCH_TABLE_OCT4: [u16; 12] = [
    0x1000, 0x10F4, 0x11F6, 0x1307, 0x1429, 0x155C, 0x16A1, 0x17FA, 0x1966, 0x1AE9, 0x1C82, 0x1E35,
];

/// 128-entry constant-power quarter-sine pan LUT (see `libakao` for derivation).
const PAN_SIN_LUT: [u8; 128] = [
    0, 2, 3, 5, 6, 8, 9, 11, 13, 14, 16, 17, 19, 20, 22, 23, 25, 27, 28, 30, 31, 33, 34, 36, 37,
    39, 40, 42, 43, 45, 46, 48, 49, 51, 52, 53, 55, 56, 58, 59, 60, 62, 63, 65, 66, 67, 69, 70, 71,
    72, 74, 75, 76, 78, 79, 80, 81, 82, 84, 85, 86, 87, 88, 89, 90, 91, 92, 93, 95, 96, 97, 98, 99,
    100, 101, 101, 102, 103, 104, 105, 106, 107, 108, 108, 109, 110, 111, 111, 112, 113, 114, 114,
    115, 116, 116, 117, 117, 118, 119, 119, 120, 120, 121, 121, 122, 122, 122, 123, 123, 124, 124,
    124, 125, 125, 125, 125, 126, 126, 126, 126, 126, 127, 127, 127, 127, 127, 127, 127,
];

/// Configuration for [`KdtPlayer`] rendering and hardware-faithful fade-out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RenderConfig {
    /// Full sequence loops to play before fade-out (`0` = play once, then tail).
    pub target_loops: u32,
    /// Per-sequence voice-volume fade-out duration in seconds (default `8.0`).
    pub fade_seconds: f64,
    /// Post-fade release / reverb decay tail in seconds (default `2.0`).
    pub tail_seconds: f64,
    /// Hard safety ceiling on total rendered duration in seconds (default `300.0`).
    pub max_seconds: f64,
    /// Output PCM sample rate in Hz (default `44100`, PS1 SPU native rate).
    pub sample_rate: u32,
    /// Whether SPU reverb master write is enabled (default `true`).
    pub reverb_enabled: bool,
    /// Fixed PS1 SPU master volume (`MVOLL`/`MVOLR`, default `0x3FFF`). Never modified by fade-out.
    pub master_volume: u16,
    /// Hardware reverb preset programmed at startup (default [`ReverbPreset::Off`]).
    pub reverb_preset: ReverbPreset,
    /// SPU reverb return depth (`EVOLL`/`EVOLR`, default `0x2000`).
    pub reverb_depth: i16,
}

impl RenderConfig {
    /// Returns the standard BGM playback configuration (2 loops, 8 s linear fade, 2 s tail, 44.1 kHz).
    #[inline]
    #[must_use]
    pub const fn bgm_default() -> Self {
        Self {
            target_loops: 2,
            fade_seconds: 8.0,
            tail_seconds: 2.0,
            max_seconds: 300.0,
            sample_rate: PS1_SAMPLE_RATE,
            reverb_enabled: true,
            master_volume: 0x3FFF,
            reverb_preset: ReverbPreset::Off,
            reverb_depth: 0x2000,
        }
    }
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self::bgm_default()
    }
}

/// Summary and interleaved stereo PCM returned by [`KdtPlayer::render_all`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderResult {
    /// Rendered interleaved stereo 16-bit PCM (`[L0, R0, L1, R1, ...]`).
    pub samples: Vec<i16>,
    /// Total number of stereo frames rendered (`samples.len() / 2`).
    pub frames_rendered: u64,
    /// Number of completed sequence loops observed during playback.
    pub loops_completed: u32,
    /// `true` if playback completed a fade-out cycle.
    pub faded_out: bool,
    /// `true` if all tracks halted naturally before hitting `max_seconds`.
    pub reached_end: bool,
}

/// Read-only runtime snapshot of a KDT logical channel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChannelSnapshot {
    /// Whether any track is currently driving this channel unfinished.
    pub active: bool,
    /// Whether this channel is currently muted (via mute mask or solo mask).
    pub muted: bool,
    /// Current program (`0x49`) on this channel.
    pub program: u8,
    /// Current bank (`0x48`) on this channel.
    pub bank: u8,
    /// Current channel volume (CC7, `0..=127`).
    pub volume: u8,
    /// Current channel expression (CC11, `0..=127`).
    pub expression: u8,
    /// Current channel pan (CC10, `0..=127`, `64` = center).
    pub pan: u8,
    /// Whether sustain pedal (CC64) is held on this channel.
    pub sustain: bool,
    /// Hardware voice currently sounding this channel, if any.
    pub voice: Option<usize>,
    /// Most recent note key triggered on this channel, if any.
    pub key: Option<u8>,
    /// Most recently computed PS1 SPU voice pitch register.
    pub pitch: u16,
    /// Live hardware voice state from `libspu2`, if a hardware voice is assigned.
    pub voice_state: Option<VoiceState>,
}

#[derive(Debug, Clone, Copy)]
struct ChannelState {
    program: u8,
    bank: u8,
    volume: u8,
    expression: u8,
    pan: u8,
    sustain: bool,
    last_key: Option<u8>,
    last_pitch: u16,
}

impl Default for ChannelState {
    fn default() -> Self {
        Self {
            program: 0,
            bank: 0,
            volume: 127,
            expression: 127,
            pan: 64,
            sustain: false,
            last_key: None,
            last_pitch: 0x1000,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct TrackRuntime {
    event_index: usize,
    wait_ticks: u32,
    tempo_acc: u16,
    tempo: u16,
    channel: usize,
    last_note: Option<u8>,
    finished: bool,
}

#[derive(Debug, Clone, Copy)]
struct ActiveVoice {
    channel: usize,
    key: u8,
    velocity: u8,
    tone: ToneAttr,
    prog_mvol: u8,
    prog_mpan: u8,
    priority: u8,
    vol_l: i16,
    vol_r: i16,
    released: bool,
    sustained: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PlayPhase {
    Playing,
    Fading,
    Tail,
    Finished,
}

/// PlayStation 1 Konami KDT music player driving [`libspu2::Device`] in PS1 SPU mode.
pub struct KdtPlayer {
    sequence: KdtSequence,
    bank: VabBank,
    config: RenderConfig,
    device: Device,
    upload: VabUpload,
    tracks: Vec<TrackRuntime>,
    channels: [ChannelState; MAX_CHANNELS],
    voices: [Option<ActiveVoice>; VOICES_PER_CORE],
    voice_age: [u64; VOICES_PER_CORE],
    mute_mask: u16,
    solo_mask: u16,
    reverb_active: bool,
    timer_ticks: u64,
    sample_accum_num: u64,
    samples_per_tick_num: u64,
    samples_per_tick_den: u64,
    sequence_volume_fp16: u32,
    initial_sequence_volume_fp16: u32,
    fade_step_fp16: u32,
    fade_elapsed_60hz: u32,
    fade_total_60hz: u32,
    fade_accum_cycles: u64,
    tail_frames_remaining: u64,
    max_frames: u64,
    frames_rendered: u64,
    loops_completed: u32,
    faded_out: bool,
    reached_end: bool,
    phase: PlayPhase,
}

impl fmt::Debug for KdtPlayer {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("KdtPlayer")
            .field("version", &self.sequence.version)
            .field("tracks", &self.sequence.track_count)
            .field("phase", &self.phase)
            .field("frames_rendered", &self.frames_rendered)
            .field("loops_completed", &self.loops_completed)
            .field("mute_mask", &self.mute_mask)
            .field("solo_mask", &self.solo_mask)
            .field("reverb_active", &self.reverb_active)
            .finish()
    }
}

impl KdtPlayer {
    /// Creates a new [`KdtPlayer`], uploads `bank` to a fresh [`Device::new_ps1`],
    /// programs the configured PS1 SPU reverb preset, and initializes all tracks.
    pub fn new(sequence: KdtSequence, bank: VabBank, config: RenderConfig) -> Result<Self> {
        if config.sample_rate == 0 {
            return Err(Error::InvalidArgument("sample_rate must be non-zero"));
        }
        if config.fade_seconds < 0.0 || !config.fade_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "fade_seconds must be non-negative and finite",
            ));
        }
        if config.tail_seconds < 0.0 || !config.tail_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "tail_seconds must be non-negative and finite",
            ));
        }
        if config.max_seconds <= 0.0 || !config.max_seconds.is_finite() {
            return Err(Error::InvalidArgument(
                "max_seconds must be positive and finite",
            ));
        }

        let mut device = Device::new_ps1();
        if config.sample_rate != PS1_SAMPLE_RATE {
            device.set_output_sample_rate(config.sample_rate)?;
        }
        let upload = bank.upload_to_spu(&mut device)?;

        let samples_per_tick_num = u64::from(config.sample_rate) * u64::from(KDT_RC2_DIVIDER);
        let samples_per_tick_den = u64::from(KDT_RC2_CLOCK_HZ);
        let fade_total_60hz = ((config.fade_seconds * 60.0) + 0.5) as u32;
        let max_frames = (config.max_seconds * f64::from(config.sample_rate)) as u64;
        let initial_sequence_volume_fp16: u32 = 127 << 16;
        let fade_step_fp16 = initial_sequence_volume_fp16
            .checked_div(fade_total_60hz)
            .map_or(initial_sequence_volume_fp16, |step| step.max(1));

        let mut player = Self {
            sequence,
            bank,
            config,
            device,
            upload,
            tracks: Vec::new(),
            channels: [ChannelState::default(); MAX_CHANNELS],
            voices: [None; VOICES_PER_CORE],
            voice_age: [0; VOICES_PER_CORE],
            mute_mask: 0,
            solo_mask: 0,
            reverb_active: false,
            timer_ticks: 0,
            sample_accum_num: 0,
            samples_per_tick_num,
            samples_per_tick_den,
            sequence_volume_fp16: initial_sequence_volume_fp16,
            initial_sequence_volume_fp16,
            fade_step_fp16,
            fade_elapsed_60hz: 0,
            fade_total_60hz,
            fade_accum_cycles: 0,
            tail_frames_remaining: 0,
            max_frames,
            frames_rendered: 0,
            loops_completed: 0,
            faded_out: false,
            reached_end: false,
            phase: PlayPhase::Playing,
        };
        player.reset_tracks();
        player.init_spu()?;
        Ok(player)
    }

    /// Resets playback state, tracks, channels, and SPU voice registers back to
    /// the start of the sequence **without** re-uploading VAB ADPCM RAM.
    pub fn reset(&mut self) -> Result<()> {
        self.device.write_ps1_register(ps1::KOFF_LO, 0xFFFF)?;
        self.device.write_ps1_register(ps1::KOFF_HI, 0x00FF)?;
        for v in 0..VOICES_PER_CORE {
            let base = (v as u32) * ps1::VOICE_STRIDE;
            self.device.write_ps1_register(base + ps1::V_VOLL, 0)?;
            self.device.write_ps1_register(base + ps1::V_VOLR, 0)?;
            self.device.write_ps1_register(base + ps1::V_PITCH, 0)?;
            self.device.write_ps1_register(base + ps1::V_ADSR1, 0)?;
            self.device.write_ps1_register(base + ps1::V_ADSR2, 0)?;
        }
        self.device.write_ps1_register(ps1::PMON_LO, 0)?;
        self.device.write_ps1_register(ps1::PMON_HI, 0)?;
        self.device.write_ps1_register(ps1::NON_LO, 0)?;
        self.device.write_ps1_register(ps1::NON_HI, 0)?;
        self.device.write_ps1_register(ps1::EON_LO, 0)?;
        self.device.write_ps1_register(ps1::EON_HI, 0)?;

        self.channels = [ChannelState::default(); MAX_CHANNELS];
        self.voices = [None; VOICES_PER_CORE];
        self.voice_age = [0; VOICES_PER_CORE];
        self.timer_ticks = 0;
        self.sample_accum_num = 0;
        self.sequence_volume_fp16 = self.initial_sequence_volume_fp16;
        self.fade_elapsed_60hz = 0;
        self.fade_accum_cycles = 0;
        self.tail_frames_remaining = 0;
        self.frames_rendered = 0;
        self.loops_completed = 0;
        self.faded_out = false;
        self.reached_end = false;
        self.phase = PlayPhase::Playing;
        self.reset_tracks();
        self.init_spu()
    }

    fn reset_tracks(&mut self) {
        self.tracks = self
            .sequence
            .tracks
            .iter()
            .enumerate()
            .map(|(idx, track)| TrackRuntime {
                event_index: 0,
                wait_ticks: track.lead_delta,
                tempo_acc: 0,
                tempo: DEFAULT_TEMPO,
                channel: idx % MAX_CHANNELS,
                last_note: None,
                finished: track.events.is_empty(),
            })
            .collect();
    }

    fn init_spu(&mut self) -> Result<()> {
        let preset = self.config.reverb_preset;
        let work_size = preset.work_area_size() as usize;
        let bank_end = (self.upload.base_addr as usize).saturating_add(self.bank.bodies.len());
        let work_start = PS1_RAM_SIZE.saturating_sub(work_size) & !7;
        self.reverb_active = self.config.reverb_enabled
            && preset != ReverbPreset::Off
            && work_size > 0
            && work_start >= bank_end;
        let params = if self.reverb_active {
            preset.params()
        } else {
            ReverbParams::ZERO
        };
        let regs = params.to_ps1_registers();
        for (idx, &word) in regs.iter().enumerate() {
            self.device
                .write_ps1_register(ps1::RVB_BASE + (idx as u32) * 2, word)?;
        }
        let work_addr_reg = if self.reverb_active {
            (work_start >> 3) as u16
        } else {
            ((PS1_RAM_SIZE - 8) >> 3) as u16
        };
        self.device
            .write_ps1_register(ps1::RVB_WORK_ADDR, work_addr_reg)?;

        let mut cnt = CONTROL_ENABLE | CONTROL_UNMUTE;
        if self.reverb_active {
            cnt |= CONTROL_REVERB;
        }
        self.device.write_ps1_register(ps1::SPUCNT, cnt)?;
        self.device
            .write_ps1_register(ps1::MVOLL, self.config.master_volume & 0x7FFF)?;
        self.device
            .write_ps1_register(ps1::MVOLR, self.config.master_volume & 0x7FFF)?;
        let depth = if self.reverb_active {
            self.config.reverb_depth
        } else {
            0
        };
        self.device.write_ps1_register(ps1::EVOLL, depth as u16)?;
        self.device.write_ps1_register(ps1::EVOLR, depth as u16)?;
        Ok(())
    }

    /// Returns a read-only reference to the underlying [`libspu2::Device`].
    #[inline]
    #[must_use]
    pub fn device(&self) -> &Device {
        &self.device
    }

    /// Returns a read-only reference to the loaded [`KdtSequence`].
    #[inline]
    #[must_use]
    pub fn sequence(&self) -> &KdtSequence {
        &self.sequence
    }

    /// Returns a read-only reference to the loaded [`VabBank`].
    #[inline]
    #[must_use]
    pub fn bank(&self) -> &VabBank {
        &self.bank
    }

    /// Returns the active [`RenderConfig`].
    #[inline]
    #[must_use]
    pub fn config(&self) -> RenderConfig {
        self.config
    }

    /// Returns the logical channel count (`16`).
    #[inline]
    #[must_use]
    pub const fn channel_count(&self) -> usize {
        MAX_CHANNELS
    }

    /// Returns `true` once playback (including fade-out and tail) has finished.
    #[inline]
    #[must_use]
    pub fn is_finished(&self) -> bool {
        self.phase == PlayPhase::Finished
    }

    /// Returns `true` while the player is actively ramping down during fade-out.
    #[inline]
    #[must_use]
    pub fn is_fading(&self) -> bool {
        self.phase == PlayPhase::Fading
    }

    /// Returns the elapsed playback time in seconds (`frames_rendered / sample_rate`).
    #[inline]
    #[must_use]
    pub fn elapsed_seconds(&self) -> f64 {
        (self.frames_rendered as f64) / f64::from(self.config.sample_rate)
    }

    /// Returns the current per-sequence volume scalar in `0..=127`.
    #[inline]
    #[must_use]
    pub fn sequence_volume(&self) -> u8 {
        (self.sequence_volume_fp16 >> 16).min(127) as u8
    }

    /// Returns the number of completed sequence loops observed so far.
    #[inline]
    #[must_use]
    pub fn loops_completed(&self) -> u32 {
        self.loops_completed
    }

    /// Returns the cumulative number of stereo frames rendered so far.
    #[inline]
    #[must_use]
    pub fn frames_rendered(&self) -> u64 {
        self.frames_rendered
    }

    /// Mutes or unmutes a single logical channel (`0..16`).
    pub fn set_channel_muted(&mut self, channel: usize, muted: bool) {
        if channel >= MAX_CHANNELS {
            return;
        }
        let bit = 1u16 << channel;
        if muted {
            self.mute_mask |= bit;
        } else {
            self.mute_mask &= !bit;
        }
        self.refresh_channel_voices(channel);
    }

    /// Replaces the channel mute bitmask (bit `i` = channel `i` muted).
    pub fn set_mute_mask(&mut self, mask: u16) {
        self.mute_mask = mask;
        for ch in 0..MAX_CHANNELS {
            self.refresh_channel_voices(ch);
        }
    }

    /// Returns the current channel mute bitmask.
    #[inline]
    #[must_use]
    pub const fn mute_mask(&self) -> u16 {
        self.mute_mask
    }

    /// Sets the channel solo bitmask (`0` disables solo; non-zero mutes all channels not in `mask`).
    pub fn set_solo_mask(&mut self, mask: u16) {
        self.solo_mask = mask;
        for ch in 0..MAX_CHANNELS {
            self.refresh_channel_voices(ch);
        }
    }

    /// Returns the current channel solo bitmask.
    #[inline]
    #[must_use]
    pub const fn solo_mask(&self) -> u16 {
        self.solo_mask
    }

    #[inline]
    const fn is_channel_effectively_muted(&self, channel: usize) -> bool {
        if channel >= MAX_CHANNELS {
            return true;
        }
        let bit = 1u16 << channel;
        if (self.mute_mask & bit) != 0 {
            return true;
        }
        self.solo_mask != 0 && (self.solo_mask & bit) == 0
    }

    /// Returns a snapshot of `channel`'s current state, or `None` if out of range.
    #[must_use]
    pub fn channel_snapshot(&self, channel: usize) -> Option<ChannelSnapshot> {
        if channel >= MAX_CHANNELS {
            return None;
        }
        let ch = &self.channels[channel];
        let voice = self
            .voices
            .iter()
            .position(|v| v.is_some_and(|a| a.channel == channel && !a.released));
        let voice_state = voice.and_then(|v| self.device.inspect_voice(0, v).ok());
        let active = self
            .tracks
            .iter()
            .any(|t| t.channel == channel && !t.finished)
            || voice.is_some();
        Some(ChannelSnapshot {
            active,
            muted: self.is_channel_effectively_muted(channel),
            program: ch.program,
            bank: ch.bank,
            volume: ch.volume,
            expression: ch.expression,
            pan: ch.pan,
            sustain: ch.sustain,
            voice,
            key: ch.last_key,
            pitch: ch.last_pitch,
            voice_state,
        })
    }

    /// Manually triggers a per-sequence fade-out over [`RenderConfig::fade_seconds`].
    pub fn start_fade_out(&mut self) {
        if self.phase == PlayPhase::Playing {
            if self.fade_total_60hz == 0 {
                self.sequence_volume_fp16 = 0;
                self.faded_out = true;
                self.enter_tail();
            } else {
                self.phase = PlayPhase::Fading;
                self.fade_elapsed_60hz = 0;
                self.fade_accum_cycles = 0;
            }
        }
    }

    /// Renders interleaved stereo `i16` PCM into `out`, returning the number of
    /// stereo frames rendered (up to `out.len() / 2`).
    ///
    /// Returns [`Error::InvalidArgument`] if `out.len()` is odd.
    pub fn render_samples(&mut self, out: &mut [i16]) -> Result<usize> {
        if out.len() % 2 != 0 {
            return Err(Error::InvalidArgument(
                "interleaved stereo buffer length must be even",
            ));
        }
        let total_frames = out.len() / 2;
        let mut rendered = 0usize;
        while rendered < total_frames && self.phase != PlayPhase::Finished {
            let to_render = self.advance_until_renderable(total_frames - rendered)?;
            if to_render == 0 {
                break;
            }
            let start = rendered * 2;
            let end = (rendered + to_render) * 2;
            self.device.render_native(&mut out[start..end]);
            rendered += to_render;
            self.commit_rendered_frames(to_render);
        }
        Ok(rendered)
    }

    /// Renders stereo frames into `out`, returning the number of frames rendered.
    pub fn render_frames(&mut self, out: &mut [StereoI16]) -> Result<usize> {
        let mut chunk = [0i16; 512];
        let mut written = 0usize;
        while written < out.len() && !self.is_finished() {
            let n = (out.len() - written).min(256);
            let got = self.render_samples(&mut chunk[..n * 2])?;
            if got == 0 {
                break;
            }
            for i in 0..got {
                out[written + i] = StereoI16 {
                    left: chunk[i * 2],
                    right: chunk[i * 2 + 1],
                };
            }
            written += got;
        }
        Ok(written)
    }

    /// Renders planar stereo `f32` PCM (`[-1.0, 1.0]`) directly into `left` and
    /// `right` with zero heap allocation, returning the number of stereo frames rendered.
    ///
    /// Returns [`Error::InvalidArgument`] if `left.len() != right.len()`.
    pub fn render_f32_planar(&mut self, left: &mut [f32], right: &mut [f32]) -> Result<usize> {
        if left.len() != right.len() {
            return Err(Error::InvalidArgument(
                "planar left and right buffers must have equal length",
            ));
        }
        let total_frames = left.len();
        let mut rendered = 0usize;
        while rendered < total_frames && self.phase != PlayPhase::Finished {
            let to_render = self.advance_until_renderable(total_frames - rendered)?;
            if to_render == 0 {
                break;
            }
            let end = rendered + to_render;
            self.device
                .render_f32_planar(&mut left[rendered..end], &mut right[rendered..end])?;
            rendered += to_render;
            self.commit_rendered_frames(to_render);
        }
        Ok(rendered)
    }

    /// Renders interleaved stereo `f32` PCM (`[L0, R0, L1, R1, ...]`) into `out`
    /// with zero heap allocation, returning the number of stereo frames rendered.
    ///
    /// Returns [`Error::InvalidArgument`] if `out.len()` is odd.
    pub fn render_f32_interleaved(&mut self, out: &mut [f32]) -> Result<usize> {
        if out.len() % 2 != 0 {
            return Err(Error::InvalidArgument(
                "interleaved stereo buffer length must be even",
            ));
        }
        let total_frames = out.len() / 2;
        let mut rendered = 0usize;
        let mut left_buf = [0.0f32; 256];
        let mut right_buf = [0.0f32; 256];
        while rendered < total_frames && self.phase != PlayPhase::Finished {
            let n = (total_frames - rendered).min(256);
            let got = self.render_f32_planar(&mut left_buf[..n], &mut right_buf[..n])?;
            if got == 0 {
                break;
            }
            for i in 0..got {
                let dst = (rendered + i) * 2;
                out[dst] = left_buf[i];
                out[dst + 1] = right_buf[i];
            }
            rendered += got;
        }
        Ok(rendered)
    }

    /// Renders the entire sequence (respecting `target_loops`, `fade_seconds`,
    /// `tail_seconds`, and `max_seconds`) into a [`RenderResult`].
    pub fn render_all(&mut self) -> Result<RenderResult> {
        let mut samples = Vec::new();
        let mut buf = [0i16; 2048];
        while !self.is_finished() {
            let frames = self.render_samples(&mut buf)?;
            if frames == 0 {
                break;
            }
            samples.extend_from_slice(&buf[..frames * 2]);
        }
        Ok(RenderResult {
            frames_rendered: self.frames_rendered,
            samples,
            loops_completed: self.loops_completed,
            faded_out: self.faded_out,
            reached_end: self.reached_end,
        })
    }

    fn advance_until_renderable(&mut self, max_wanted: usize) -> Result<usize> {
        loop {
            if self.phase == PlayPhase::Finished || max_wanted == 0 {
                return Ok(0);
            }
            if self.frames_rendered >= self.max_frames {
                self.phase = PlayPhase::Finished;
                return Ok(0);
            }
            let remaining_cap = (self.max_frames - self.frames_rendered) as usize;
            if self.phase == PlayPhase::Tail {
                let to_render = max_wanted
                    .min(self.tail_frames_remaining as usize)
                    .min(remaining_cap);
                if to_render == 0 {
                    self.phase = PlayPhase::Finished;
                }
                return Ok(to_render);
            }
            while self.sample_accum_num < self.samples_per_tick_den {
                self.step_hardware_timer()?;
                if self.phase == PlayPhase::Tail || self.phase == PlayPhase::Finished {
                    break;
                }
                self.sample_accum_num += self.samples_per_tick_num;
            }
            if self.phase == PlayPhase::Tail || self.phase == PlayPhase::Finished {
                continue;
            }
            let step_frames = (self.sample_accum_num / self.samples_per_tick_den) as usize;
            return Ok(step_frames.min(max_wanted).min(remaining_cap));
        }
    }

    fn commit_rendered_frames(&mut self, frames: usize) {
        self.frames_rendered += frames as u64;
        if self.phase == PlayPhase::Tail {
            self.tail_frames_remaining = self.tail_frames_remaining.saturating_sub(frames as u64);
            if self.tail_frames_remaining == 0 {
                self.phase = PlayPhase::Finished;
            }
        } else {
            self.sample_accum_num -= (frames as u64) * self.samples_per_tick_den;
        }
    }

    fn step_hardware_timer(&mut self) -> Result<()> {
        // 60 Hz fade-out tick: one step per 70,560 RC2 clock cycles
        // (4,233,600 / 60), i.e. every ~9.63 Root Counter 2 ticks.
        if self.phase == PlayPhase::Fading {
            self.fade_accum_cycles += u64::from(KDT_RC2_DIVIDER);
            if self.fade_accum_cycles >= 70_560 {
                self.fade_accum_cycles -= 70_560;
                self.step_fade_60hz()?;
                if self.phase != PlayPhase::Fading {
                    self.timer_ticks += 1;
                    return Ok(());
                }
            }
        }

        let mut kon_mask: u32 = 0;
        let mut koff_mask: u32 = 0;
        for track_idx in 0..self.tracks.len() {
            if self.tracks[track_idx].finished {
                continue;
            }
            // Per-track 8-bit fractional tempo accumulator (FUN_800e38a4).
            let acc = self.tracks[track_idx]
                .tempo_acc
                .saturating_add(self.tracks[track_idx].tempo);
            if acc <= 0xFF {
                self.tracks[track_idx].tempo_acc = acc;
                continue;
            }
            self.tracks[track_idx].tempo_acc = acc - 0x100;

            if self.tracks[track_idx].wait_ticks > 0 {
                self.tracks[track_idx].wait_ticks -= 1;
                continue;
            }
            while !self.tracks[track_idx].finished && self.tracks[track_idx].wait_ticks == 0 {
                let ev_idx = self.tracks[track_idx].event_index;
                let Some(&ev) = self.sequence.tracks[track_idx].events.get(ev_idx) else {
                    self.tracks[track_idx].finished = true;
                    break;
                };
                self.tracks[track_idx].event_index += 1;
                self.apply_command(track_idx, ev.command, &mut kon_mask, &mut koff_mask)?;
                if ev.delta > 0 {
                    self.tracks[track_idx].wait_ticks = ev.delta;
                    break;
                }
            }
            if self.tracks[track_idx].wait_ticks > 0 {
                self.tracks[track_idx].wait_ticks -= 1;
            }
        }
        if koff_mask != 0 {
            self.device
                .write_ps1_register(ps1::KOFF_LO, (koff_mask & 0xFFFF) as u16)?;
            self.device
                .write_ps1_register(ps1::KOFF_HI, ((koff_mask >> 16) & 0xFF) as u16)?;
        }
        if kon_mask != 0 {
            self.device
                .write_ps1_register(ps1::KON_LO, (kon_mask & 0xFFFF) as u16)?;
            self.device
                .write_ps1_register(ps1::KON_HI, ((kon_mask >> 16) & 0xFF) as u16)?;
        }
        self.timer_ticks += 1;

        if !self.tracks.is_empty() && self.tracks.iter().all(|t| t.finished) {
            self.loops_completed = self.loops_completed.saturating_add(1);
            if self.phase == PlayPhase::Fading {
                self.reset_tracks();
            } else if self.config.target_loops > 0
                && self.loops_completed >= self.config.target_loops
            {
                self.start_fade_out();
                self.reset_tracks();
            } else if self.config.target_loops == 0 {
                self.reached_end = true;
                self.enter_tail();
            } else {
                self.reset_tracks();
            }
        }
        Ok(())
    }

    fn step_fade_60hz(&mut self) -> Result<()> {
        self.fade_elapsed_60hz += 1;
        let old_int = self.sequence_volume_fp16 >> 16;
        if self.fade_elapsed_60hz >= self.fade_total_60hz {
            self.sequence_volume_fp16 = 0;
        } else {
            // Integer fixed-point 60 Hz linear fade (zero floating-point math).
            self.sequence_volume_fp16 = self
                .sequence_volume_fp16
                .saturating_sub(self.fade_step_fp16);
        }
        let new_int = self.sequence_volume_fp16 >> 16;
        if new_int != old_int {
            for ch in 0..MAX_CHANNELS {
                self.refresh_channel_voices(ch);
            }
        }
        if self.sequence_volume_fp16 == 0 {
            self.faded_out = true;
            self.enter_tail();
        }
        Ok(())
    }

    fn enter_tail(&mut self) {
        // Key off all 24 PS1 SPU voices (leave MVOLL/MVOLR untouched so
        // reverb/release decay naturally).
        let _ = self.device.write_ps1_register(ps1::KOFF_LO, 0xFFFF);
        let _ = self.device.write_ps1_register(ps1::KOFF_HI, 0x00FF);
        for a in self.voices.iter_mut().flatten() {
            a.released = true;
            a.sustained = false;
        }
        let tail_frames = (self.config.tail_seconds * f64::from(self.config.sample_rate)) as u64;
        if tail_frames == 0 {
            self.phase = PlayPhase::Finished;
        } else {
            self.tail_frames_remaining = tail_frames;
            self.phase = PlayPhase::Tail;
        }
    }

    fn apply_command(
        &mut self,
        track_idx: usize,
        command: KdtCommand,
        kon_mask: &mut u32,
        koff_mask: &mut u32,
    ) -> Result<()> {
        match command {
            KdtCommand::NoteOn { key, velocity } => {
                let chan = self.tracks[track_idx].channel;
                self.tracks[track_idx].last_note = Some(key);
                self.note_on(chan, key, velocity, kon_mask)?;
            }
            KdtCommand::NoteOff { key } => {
                let chan = self.tracks[track_idx].channel;
                self.note_off(chan, key, koff_mask);
            }
            KdtCommand::ChannelSelect(ch) => {
                self.tracks[track_idx].channel = usize::from(ch & 0x0F);
            }
            KdtCommand::Tempo(tempo) => {
                self.tracks[track_idx].tempo = tempo.max(2);
            }
            KdtCommand::BankParam(bank) => {
                let chan = self.tracks[track_idx].channel;
                self.channels[chan].bank = bank;
            }
            KdtCommand::ProgramChange(prog) => {
                let chan = self.tracks[track_idx].channel;
                self.channels[chan].program = prog;
            }
            KdtCommand::RunningNoteOff => {
                let chan = self.tracks[track_idx].channel;
                if let Some(key) = self.tracks[track_idx].last_note.take() {
                    self.note_off(chan, key, koff_mask);
                }
            }
            KdtCommand::ControlChange { controller, value } => {
                let chan = self.tracks[track_idx].channel;
                self.apply_cc(chan, controller, value, koff_mask);
            }
            KdtCommand::EndOfTrack => {
                self.tracks[track_idx].finished = true;
            }
        }
        Ok(())
    }

    fn apply_cc(&mut self, chan: usize, cc: u8, val: u8, koff_mask: &mut u32) {
        match cc {
            7 => {
                self.channels[chan].volume = val;
                self.refresh_channel_voices(chan);
            }
            10 => {
                self.channels[chan].pan = val;
                self.refresh_channel_voices(chan);
            }
            11 => {
                self.channels[chan].expression = val;
                self.refresh_channel_voices(chan);
            }
            64 => {
                let down = val >= 64;
                let was_down = self.channels[chan].sustain;
                self.channels[chan].sustain = down;
                if was_down && !down {
                    self.release_sustained(chan, koff_mask);
                }
            }
            120 | 123 => {
                self.release_channel(chan, koff_mask);
            }
            121 => {
                self.channels[chan].expression = 127;
                self.channels[chan].volume = 127;
                let was_down = self.channels[chan].sustain;
                self.channels[chan].sustain = false;
                if was_down {
                    self.release_sustained(chan, koff_mask);
                }
                self.refresh_channel_voices(chan);
            }
            _ => {}
        }
    }

    fn note_on(&mut self, chan: usize, key: u8, velocity: u8, kon_mask: &mut u32) -> Result<()> {
        self.channels[chan].last_key = Some(key);
        if self.is_channel_effectively_muted(chan) {
            return Ok(());
        }
        let program = self.channels[chan].program as usize;
        // Resolve matching tones first so the bank borrow ends before
        // mutable voice allocation / register writes.
        let mut matched: [(ToneAttr, u32, u8); 16] = [(
            ToneAttr {
                priority: 0,
                mode: 0,
                volume: 0,
                pan: 0,
                center: 0,
                shift: 0,
                min: 0,
                max: 0,
                adsr1: 0,
                adsr2: 0,
                program: 0,
                vag: 0,
            },
            0,
            0,
        ); 16];
        let mut matched_count = 0usize;
        {
            let Some((prog, tones)) = self.bank.program_block(program) else {
                return Ok(());
            };
            let ntones = usize::from(prog.tones).min(16).min(tones.len());
            for tone in tones.iter().take(ntones) {
                if key < tone.min || key > tone.max || tone.vag <= 0 {
                    continue;
                }
                let slot_id = tone.vag as usize;
                let Some(start_addr) = self.upload.vag_addrs.get(slot_id).copied().flatten() else {
                    continue;
                };
                let priority = if tone.priority != 0 {
                    tone.priority
                } else {
                    prog.priority
                };
                matched[matched_count] = (*tone, start_addr, priority);
                matched_count += 1;
            }
            if matched_count == 0 {
                return Ok(());
            }
        }
        let (prog_mvol, prog_mpan) = self
            .bank
            .program_block(program)
            .map_or((127, 64), |(p, _)| (p.master_volume, p.master_pan));
        let ch = self.channels[chan];
        let bank_mvol = self.bank.header.master_volume;
        let seq_gain = (self.sequence_volume_fp16 as f32) / (127.0 * 65536.0);

        for (tone, start_addr, priority) in matched.iter().take(matched_count) {
            let tone = *tone;
            let start_addr = *start_addr;
            let priority = *priority;
            let voice = self.allocate_voice(priority);
            let pitch = pitch_reg(key, tone.center, tone.shift);
            let (vol_l, vol_r) = voice_volumes(
                velocity, &tone, prog_mvol, prog_mpan, bank_mvol, ch, seq_gain,
            );
            let reverb_send = self.reverb_active && ((tone.mode & TONE_MODE_REVERB) != 0);
            let base = (voice as u32) * ps1::VOICE_STRIDE;
            self.device
                .write_ps1_register(base + ps1::V_START_ADDR, (start_addr >> 3) as u16)?;
            self.device.write_ps1_register(base + ps1::V_PITCH, pitch)?;
            self.device
                .write_ps1_register(base + ps1::V_ADSR1, tone.adsr1)?;
            self.device
                .write_ps1_register(base + ps1::V_ADSR2, tone.adsr2)?;
            self.device
                .write_ps1_register(base + ps1::V_VOLL, vol_l as u16)?;
            self.device
                .write_ps1_register(base + ps1::V_VOLR, vol_r as u16)?;
            if reverb_send {
                self.set_eon(voice, true)?;
            }
            self.channels[chan].last_pitch = pitch;
            self.voices[voice] = Some(ActiveVoice {
                channel: chan,
                key,
                velocity,
                tone,
                prog_mvol,
                prog_mpan,
                priority,
                vol_l,
                vol_r,
                released: false,
                sustained: false,
            });
            self.voice_age[voice] = self.timer_ticks;
            *kon_mask |= 1u32 << voice;
        }
        Ok(())
    }

    fn note_off(&mut self, chan: usize, key: u8, koff_mask: &mut u32) {
        let sustain = self.channels[chan].sustain;
        for (idx, entry) in self.voices.iter_mut().enumerate() {
            let Some(v) = entry else { continue };
            if !v.released && v.channel == chan && v.key == key {
                if sustain {
                    v.sustained = true;
                } else {
                    *koff_mask |= 1u32 << idx;
                    v.released = true;
                    v.sustained = false;
                }
            }
        }
    }

    fn release_sustained(&mut self, chan: usize, koff_mask: &mut u32) {
        for (idx, entry) in self.voices.iter_mut().enumerate() {
            let Some(v) = entry else { continue };
            if v.channel == chan && v.sustained && !v.released {
                *koff_mask |= 1u32 << idx;
                v.released = true;
                v.sustained = false;
            }
        }
    }

    fn release_channel(&mut self, chan: usize, koff_mask: &mut u32) {
        for (idx, entry) in self.voices.iter_mut().enumerate() {
            let Some(v) = entry else { continue };
            if v.channel == chan && !v.released {
                *koff_mask |= 1u32 << idx;
                v.released = true;
                v.sustained = false;
            }
        }
    }

    /// 24-voice priority allocator: free voice first, then released voices,
    /// then lowest-priority oldest voice (hardware `SpuSetAnyVoice`-style steal).
    fn allocate_voice(&mut self, priority: u8) -> usize {
        if let Some(free) = self.voices.iter().position(|v| v.is_none()) {
            return free;
        }
        if let Some(rel) = self
            .voices
            .iter()
            .position(|v| v.is_some_and(|a| a.released))
        {
            return rel;
        }
        let mut best = 0usize;
        let mut best_prio = u8::MAX;
        let mut best_age = u64::MAX;
        for (idx, entry) in self.voices.iter().enumerate() {
            let Some(v) = entry else { continue };
            let age = self.voice_age[idx];
            if v.priority < best_prio || (v.priority == best_prio && age < best_age) {
                best = idx;
                best_prio = v.priority;
                best_age = age;
            }
        }
        let _ = priority;
        best
    }

    fn set_eon(&mut self, voice: usize, on: bool) -> Result<()> {
        if voice >= VOICES_PER_CORE {
            return Ok(());
        }
        let (reg, bit) = if voice < 16 {
            (ps1::EON_LO, voice as u16)
        } else {
            (ps1::EON_HI, (voice - 16) as u16)
        };
        let cur = self.device.read_ps1_register(reg)?;
        let next = if on {
            cur | (1u16 << bit)
        } else {
            cur & !(1u16 << bit)
        };
        if next != cur {
            self.device.write_ps1_register(reg, next)?;
        }
        Ok(())
    }

    fn refresh_channel_voices(&mut self, chan: usize) {
        let ch = self.channels[chan];
        let bank_mvol = self.bank.header.master_volume;
        let seq_gain = (self.sequence_volume_fp16 as f32) / (127.0 * 65536.0);
        let muted = self.is_channel_effectively_muted(chan);
        for (idx, entry) in self.voices.iter_mut().enumerate() {
            let Some(v) = entry else { continue };
            if v.channel != chan || v.released {
                continue;
            }
            let (mut vol_l, mut vol_r) = voice_volumes(
                v.velocity,
                &v.tone,
                v.prog_mvol,
                v.prog_mpan,
                bank_mvol,
                ch,
                seq_gain,
            );
            if muted {
                vol_l = 0;
                vol_r = 0;
            }
            if (vol_l, vol_r) != (v.vol_l, v.vol_r) {
                let base = (idx as u32) * ps1::VOICE_STRIDE;
                let _ = self
                    .device
                    .write_ps1_register(base + ps1::V_VOLL, vol_l as u16);
                let _ = self
                    .device
                    .write_ps1_register(base + ps1::V_VOLR, vol_r as u16);
                v.vol_l = vol_l;
                v.vol_r = vol_r;
            }
        }
    }
}

/// Computes the PS1 SPU pitch register for `key` against tone center/shift
/// using the 12-TET table with 1/128-semitone linear interpolation.
fn pitch_reg(key: u8, center: u8, shift: u8) -> u16 {
    let total_x128: i32 = (i32::from(key) - i32::from(center)) * 128 - i32::from(shift);
    let semi = total_x128.div_euclid(128);
    let frac = total_x128.rem_euclid(128) as u32;
    let octave = semi.div_euclid(12);
    let note = semi.rem_euclid(12) as usize;
    let base = u32::from(PITCH_TABLE_OCT4[note]);
    let next_note = (note + 1) % 12;
    let mut next = u32::from(PITCH_TABLE_OCT4[next_note]);
    if note == 11 {
        next *= 2;
    }
    let mut pitch = base + ((next - base) * frac) / 128;
    if octave >= 0 {
        pitch <<= octave.min(8) as u32;
    } else {
        pitch >>= (-octave).min(8) as u32;
    }
    pitch.clamp(1, 0xFFFF) as u16
}

#[allow(clippy::too_many_arguments)]
fn voice_volumes(
    velocity: u8,
    tone: &ToneAttr,
    prog_mvol: u8,
    prog_mpan: u8,
    bank_mvol: u8,
    ch: ChannelState,
    seq_gain: f32,
) -> (i16, i16) {
    let gain = (f32::from(velocity) / 127.0)
        * (f32::from(tone.volume) / 127.0)
        * (f32::from(prog_mvol) / 127.0)
        * (f32::from(bank_mvol) / 127.0)
        * (f32::from(ch.volume) / 127.0)
        * (f32::from(ch.expression) / 127.0)
        * seq_gain.clamp(0.0, 1.0);
    let pan = (i32::from(tone.pan) + (i32::from(ch.pan) - 64) + (i32::from(prog_mpan) - 64))
        .clamp(0, 127) as usize;
    let pan_l = f32::from(PAN_SIN_LUT[127 - pan]) / 127.0;
    let pan_r = f32::from(PAN_SIN_LUT[pan]) / 127.0;
    (
        (gain * pan_l * 16383.0) as i16,
        (gain * pan_r * 16383.0) as i16,
    )
}
