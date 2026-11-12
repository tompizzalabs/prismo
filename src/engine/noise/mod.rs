//! Noise engine: hammer impact noise, the hammer thump reaching the bridge,
//! and key release (damper landing) noise.
//!
//! A real hammer strike makes two distinct sounds at once (and real recordings
//! of them, e.g. the Yamaha VL1 / close-miked grands, show both clearly):
//!   * a short, bright **tick/click** as the felt first catches the strings, and
//!   * a low **thump/thock** as the hammer/string energy slams into the bridge
//!     and soundboard (the audible "thunk" of the key and body).
//! Instead of playing one broad hiss for both, this engine splits them: the tick
//! is a high-passed burst applied to the strings (it scatters into the pitched
//! sound), the thump is a low-passed burst driving the bridge/body. The damper
//! release is likewise a soft low **pad** (the felt landing) plus a faint higher
//! **scratch** of the felt brushing the strings.
//!
//! Everything related to these noises lives here: burst timers, decay envelopes,
//! low/high-pass filter states, and deterministic PRNG state. The rest of the
//! voice engine only asks the [`NoiseEngine`] for the forces it exerts each step.

use crate::engine::math::vector::Vector3;
use std::f64::consts::PI;

// ---- Hammer impact: bright tick on the strings ----
/// How long the felt/string contact tick lasts.
pub const HAMMER_CLICK_WINDOW_SECONDS: f64 = 0.007;
/// Time constant of the tick decay (a tick is a few milliseconds).
pub const HAMMER_CLICK_DECAY_SECONDS: f64 = 0.0018;
/// Strength of the tick applied to the strings, relative to the master level.
pub const HAMMER_CLICK_STRING_LEVEL: f64 = 0.55;

// ---- Hammer impact: low thump on the bridge ----
/// How long the body knock rings.
pub const HAMMER_THUMP_WINDOW_SECONDS: f64 = 0.030;
/// Time constant of the thump decay.
pub const HAMMER_THUMP_DECAY_SECONDS: f64 = 0.007;
/// Low-pass corner of the thump so it stays "thocky" rather than hissy.
pub const HAMMER_THUMP_LOW_CUTOFF_HZ: f64 = 420.0;
/// Strength of the thump driving the bridge, relative to the master level.
pub const HAMMER_THUMP_BRIDGE_LEVEL: f64 = 0.9;

// ---- Key release (damper landing) ----
/// Duration of the damper-landing noise burst.
pub const KEY_RELEASE_WINDOW_SECONDS: f64 = 0.045;
/// Time constant of the release pad decay.
pub const KEY_RELEASE_DECAY_SECONDS: f64 = 0.013;
/// Low-pass corner of the release pad.
pub const KEY_RELEASE_CUTOFF_HZ: f64 = 640.0;
/// Peak amplitude of the release pad.
pub const KEY_RELEASE_AMPLITUDE: f64 = 0.06;
/// High-pass corner of the faint felt scratch on the strings.
pub const KEY_RELEASE_SCRATCH_HIGH_CUTOFF_HZ: f64 = 2400.0;
/// Strength of the scratch relative to the release pad.
pub const KEY_RELEASE_SCRATCH_LEVEL: f64 = 0.3;

/// Multiplier used to scramble the release-noise PRNG state on each trigger.
const RELEASE_NOISE_STATE_ROTATION: u64 = 0x9E37_79B9_7F4A_7C15;
/// Offset applied to the strike seed so hammer and release noise do not share
/// the exact same PRNG sequence.
const STRIKE_SEED_SCRAMBLE: u64 = 0x5A5A_5A5A_5A5A_5A5A;

/// The forces exerted by the noise bursts for one simulation step.
#[derive(Clone, Copy, Debug)]
pub struct NoiseForces {
    /// Hammer impact "tick" force applied to each unison string during the
    /// strike coupling sweep.
    pub hammer_on_string: Vector3,
    /// Hammer "thump" force applied to the bridge on the final sweep of the step.
    pub hammer_on_bridge: Vector3,
    /// Key release (damper landing) force applied to the bridge on the final
    /// sweep of the step.
    pub release_on_bridge: Vector3,
}

/// A single shaped noise burst: deterministic white noise shaped by a one-pole
/// filter (low- or high-pass) and an exponentially decaying envelope.
struct ShapedBurst {
    window_seconds: f64,
    decay_seconds: f64,
    high_pass: bool,
    cutoff_hz: f64,
    seconds_remaining: f64,
    amplitude: f64,
    filter_state: f64,
    random_state: u64,
}

impl ShapedBurst {
    #[inline]
    fn new(
        window_seconds: f64,
        decay_seconds: f64,
        high_pass: bool,
        cutoff_hz: f64,
        seed: u64,
    ) -> Self {
        ShapedBurst {
            window_seconds,
            decay_seconds,
            high_pass,
            cutoff_hz,
            seconds_remaining: 0.0,
            amplitude: 0.0,
            filter_state: 0.0,
            random_state: seed | 1,
        }
    }

    /// (Re)start the burst with the given peak amplitude.
    #[inline]
    fn arm(&mut self, amplitude: f64) {
        self.seconds_remaining = self.window_seconds;
        self.amplitude = amplitude;
        self.filter_state = 0.0;
    }

    /// Set a new filter corner (used to brighten the tick with strike velocity).
    #[inline]
    fn set_cutoff(&mut self, cutoff_hz: f64) {
        self.cutoff_hz = cutoff_hz.max(40.0);
    }

    /// Advance the burst one step, returning the enveloped filtered sample
    /// (zero once the burst has finished).
    #[inline]
    fn advance(&mut self, step_size: f64) -> f64 {
        if self.seconds_remaining <= 0.0 || self.amplitude <= 0.0 {
            return 0.0;
        }
        self.seconds_remaining -= step_size;

        let elapsed_seconds = self.window_seconds - self.seconds_remaining.max(0.0);
        let decay_envelope = (-(elapsed_seconds / self.decay_seconds)).exp();

        let random_sample = next_noise_sample(&mut self.random_state);

        let low_pass_coefficient = 1.0 - (-2.0 * PI * self.cutoff_hz * step_size).exp();
        let low_passed =
            self.filter_state + low_pass_coefficient * (random_sample - self.filter_state);
        self.filter_state = low_passed;

        let shaped = if self.high_pass {
            random_sample - low_passed
        } else {
            low_passed
        };

        self.amplitude * shaped * decay_envelope
    }
}

/// Stochastic noise generator producing hammer impact (tick + thump) and
/// key-release (pad + scratch) bursts.
///
/// Each burst is a deterministically seeded xorshift white-noise stream shaped
/// by a one-pole filter and an exponential decay envelope.
pub struct NoiseEngine {
    hammer_level: f64,
    tick_base_cutoff: f64,
    click: ShapedBurst,
    thump: ShapedBurst,
    release_pad: ShapedBurst,
    release_scratch: ShapedBurst,
}

impl NoiseEngine {
    /// Build an idle noise engine. `hammer_level` is the master strike-noise
    /// loudness and `cutoff_hz` the nominal brightness of the tick (both are
    /// derived from the note's voicing controls).
    pub fn new(hammer_level: f64, cutoff_hz: f64) -> Self {
        let tick_base_cutoff = cutoff_hz.max(600.0);
        NoiseEngine {
            hammer_level: hammer_level.max(0.0),
            tick_base_cutoff,
            click: ShapedBurst::new(
                HAMMER_CLICK_WINDOW_SECONDS,
                HAMMER_CLICK_DECAY_SECONDS,
                true,
                tick_base_cutoff,
                1,
            ),
            thump: ShapedBurst::new(
                HAMMER_THUMP_WINDOW_SECONDS,
                HAMMER_THUMP_DECAY_SECONDS,
                false,
                HAMMER_THUMP_LOW_CUTOFF_HZ,
                0x1020_3040_5060_7080,
            ),
            release_pad: ShapedBurst::new(
                KEY_RELEASE_WINDOW_SECONDS,
                KEY_RELEASE_DECAY_SECONDS,
                false,
                KEY_RELEASE_CUTOFF_HZ,
                0x2545_F491_4F6C_DD15,
            ),
            release_scratch: ShapedBurst::new(
                KEY_RELEASE_WINDOW_SECONDS,
                KEY_RELEASE_DECAY_SECONDS * 0.5,
                true,
                KEY_RELEASE_SCRATCH_HIGH_CUTOFF_HZ,
                0xFEDC_BA98_7654_3210,
            ),
        }
    }

    /// Reset the engine for a fresh hammer strike, seeded deterministically so
    /// the impact noise is reproducible for a given note/voice. `hammer_velocity`
    /// scales the tick brightness and the thump's strength so hard blows sound
    /// punchier, soft ones softer.
    pub fn begin_strike(&mut self, noise_seed: u64, hammer_velocity: f64) {
        let velocity_factor = (hammer_velocity / 4.0).clamp(0.25, 1.6);

        self.click.random_state = (noise_seed ^ STRIKE_SEED_SCRAMBLE) | 1;
        self.thump.random_state =
            (noise_seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ 0xA5A5_5A5A_5A5A_5A5A) | 1;
        self.release_pad.seconds_remaining = 0.0;
        self.release_pad.amplitude = 0.0;
        self.release_scratch.seconds_remaining = 0.0;
        self.release_scratch.amplitude = 0.0;

        // Harder blows are both louder (thump) and brighter (tick).
        let tick_cutoff = self
            .tick_base_cutoff
            * (1.0 + 0.6 * (velocity_factor - 1.0).max(0.0));
        self.click
            .set_cutoff(tick_cutoff.clamp(600.0, 12000.0));
        self.click
            .arm(self.hammer_level * HAMMER_CLICK_STRING_LEVEL * velocity_factor);
        self.thump
            .arm(self.hammer_level * HAMMER_THUMP_BRIDGE_LEVEL * velocity_factor.sqrt());
    }

    /// Trigger the audible noise of the damper landing on the strings. The
    /// strike velocity scales the landing loudness.
    pub fn trigger_key_release(&mut self, strike_velocity: f64) {
        let velocity_scale = (strike_velocity / 4.0).clamp(0.2, 1.5);
        self.release_pad.random_state =
            self.release_pad.random_state.wrapping_add(RELEASE_NOISE_STATE_ROTATION) | 1;
        self.release_scratch.random_state =
            self.release_scratch.random_state.wrapping_add(RELEASE_NOISE_STATE_ROTATION >> 1) | 1;

        self.release_pad
            .arm(KEY_RELEASE_AMPLITUDE * velocity_scale);
        self.release_scratch.arm(
            KEY_RELEASE_AMPLITUDE * velocity_scale * KEY_RELEASE_SCRATCH_LEVEL,
        );
    }

    /// Advance every active noise burst by one step and return the forces they
    /// exert on the strings and bridge.
    pub fn advance(&mut self, step_size: f64) -> NoiseForces {
        let click_sample = self.click.advance(step_size);
        let thump_sample = self.thump.advance(step_size);
        let release_pad_sample = self.release_pad.advance(step_size);
        let release_scratch_sample = self.release_scratch.advance(step_size);

        NoiseForces {
            // The bright tick lands on the strings; the low thump drives the
            // bridge/body. Slight lateral component on each for width.
            hammer_on_string: Vector3::new(0.0, click_sample * 0.2, click_sample),
            hammer_on_bridge: Vector3::new(0.0, thump_sample * 0.18, thump_sample),
            release_on_bridge: Vector3::new(
                0.0,
                release_pad_sample * 0.15 + release_scratch_sample * 0.1,
                release_pad_sample + release_scratch_sample,
            ),
        }
    }

    /// Whether any noise burst is still playing out.
    pub fn is_active(&self) -> bool {
        self.click.seconds_remaining > 0.0
            || self.thump.seconds_remaining > 0.0
            || self.release_pad.seconds_remaining > 0.0
            || self.release_scratch.seconds_remaining > 0.0
    }
}

/// Advance a xorshift64* PRNG and return a white noise sample in `[-1, 1]`.
///
/// The stream is deterministic: identical seeds reproduce identical noise.
#[inline]
fn next_noise_sample(random_state: &mut u64) -> f64 {
    let mut state = *random_state;
    state ^= state >> 12;
    state ^= state << 25;
    state ^= state >> 27;
    *random_state = state;

    2.0 * ((state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64 / (1u64 << 53) as f64) - 1.0
}
