//! Noise engine: hammer impact noise, the hammer thump reaching the bridge,
//! and key release (damper landing) noise.
//!
//! Everything related to these noises lives here: the burst timers, decay
//! envelopes, low-pass filter states, and deterministic PRNG state. The rest
//! of the voice engine only asks the [`NoiseEngine`] for the forces it exerts
//! each step and never tracks noise bookkeeping itself.

use crate::engine::math::vector::Vector3;
use std::f64::consts::PI;

/// Duration of the hammer impact noise burst in seconds.
pub const HAMMER_NOISE_WINDOW_SECONDS: f64 = 12.0e-3;

/// Time constant for the hammer noise decay envelope in seconds.
pub const HAMMER_NOISE_DECAY_SECONDS: f64 = 2.5e-3;

/// Scales the part of the hammer thump that reaches the bridge.
pub const HAMMER_THUMP_BRIDGE_SCALE: f64 = 0.12;

/// Duration of the key release (damper landing) noise burst in seconds.
pub const KEY_RELEASE_NOISE_WINDOW_SECONDS: f64 = 0.030;

/// Time constant for the key release noise decay envelope in seconds.
pub const KEY_RELEASE_NOISE_DECAY_SECONDS: f64 = 0.009;

/// Peak amplitude of the key release noise.
pub const KEY_RELEASE_NOISE_AMPLITUDE: f64 = 0.05;

/// Cutoff frequency of the low-pass filter applied to key release noise.
pub const KEY_RELEASE_NOISE_CUTOFF_HZ: f64 = 850.0;

/// Multiplier used to scramble the release-noise PRNG state on each trigger.
const RELEASE_NOISE_STATE_ROTATION: u64 = 0x9E37_79B9_7F4A_7C15;

/// Offset applied to the strike seed so hammer and release noise do not share
/// the exact same PRNG sequence.
const STRIKE_SEED_SCRAMBLE: u64 = 0x5A5A_5A5A_5A5A_5A5A;

/// The forces exerted by the noise bursts for one simulation step.
#[derive(Clone, Copy, Debug)]
pub struct NoiseForces {
    /// Hammer impact noise force applied to each unison string during the
    /// strike coupling sweep.
    pub hammer_on_string: Vector3,
    /// Hammer thump force applied to the bridge on the final sweep of the step.
    pub hammer_on_bridge: Vector3,
    /// Key release (damper landing) force applied to the bridge on the final
    /// sweep of the step.
    pub release_on_bridge: Vector3,
}

/// Stochastic noise generator producing hammer impact and key-release bursts.
///
/// Each burst is a shaped white-noise burst: a deterministic xorshift PRNG
/// feeds a one-pole low-pass filter, which is then enveloped by a decaying
/// exponential determined by the burst's time constant.
pub struct NoiseEngine {
    hammer_noise_seconds_remaining: f64,
    hammer_noise_amplitude: f64,
    hammer_noise_cutoff_hz: f64,
    hammer_noise_filter_state: f64,
    hammer_noise_random_state: u64,
    release_noise_seconds_remaining: f64,
    release_noise_amplitude: f64,
    release_noise_filter_state: f64,
    release_noise_random_state: u64,
}

impl NoiseEngine {
    /// Build an idle noise engine for a hammer with the given impact-noise
    /// amplitude and cutoff frequency.
    pub fn new(hammer_noise_amplitude: f64, hammer_noise_cutoff_hz: f64) -> Self {
        NoiseEngine {
            hammer_noise_seconds_remaining: 0.0,
            hammer_noise_amplitude,
            hammer_noise_cutoff_hz,
            hammer_noise_filter_state: 0.0,
            hammer_noise_random_state: 1,
            release_noise_seconds_remaining: 0.0,
            release_noise_amplitude: 0.0,
            release_noise_filter_state: 0.0,
            release_noise_random_state: 0x2545_F491_4F6C_DD15,
        }
    }

    /// Reset the engine for a fresh hammer strike, seeded deterministically so
    /// the impact noise is reproducible for a given note and voice.
    pub fn begin_strike(&mut self, noise_seed: u64) {
        self.hammer_noise_seconds_remaining = HAMMER_NOISE_WINDOW_SECONDS;
        self.hammer_noise_filter_state = 0.0;
        self.hammer_noise_random_state = noise_seed | 1;

        self.release_noise_seconds_remaining = 0.0;
        self.release_noise_amplitude = 0.0;
        self.release_noise_filter_state = 0.0;
        self.release_noise_random_state = (noise_seed ^ STRIKE_SEED_SCRAMBLE) | 1;
    }

    /// Trigger the audible noise of the damper landing on the strings. The
    /// strike velocity scales the landing loudness.
    pub fn trigger_key_release(&mut self, strike_velocity: f64) {
        let velocity_scale = (strike_velocity / 4.0).clamp(0.2, 1.5);
        self.release_noise_seconds_remaining = KEY_RELEASE_NOISE_WINDOW_SECONDS;
        self.release_noise_amplitude = KEY_RELEASE_NOISE_AMPLITUDE * velocity_scale;
        self.release_noise_filter_state = 0.0;
        self.release_noise_random_state =
            self.release_noise_random_state.wrapping_add(RELEASE_NOISE_STATE_ROTATION) | 1;
    }

    /// Advance every active noise burst by one step and return the forces they
    /// exert on the strings and bridge.
    pub fn advance(&mut self, step_size: f64) -> NoiseForces {
        let hammer_force = self.advance_hammer_noise(step_size);
        let release_force = self.advance_release_noise(step_size);

        NoiseForces {
            hammer_on_string: hammer_force,
            hammer_on_bridge: Vector3::new(
                0.0,
                hammer_force.z * 0.25 * HAMMER_THUMP_BRIDGE_SCALE,
                hammer_force.z * HAMMER_THUMP_BRIDGE_SCALE,
            ),
            release_on_bridge: release_force,
        }
    }

    /// Whether any noise burst is still playing out.
    pub fn is_active(&self) -> bool {
        self.hammer_noise_seconds_remaining > 0.0
            || self.release_noise_seconds_remaining > 0.0
    }

    /// Advance the hammer impact noise burst, returning its force on the strings.
    #[inline]
    fn advance_hammer_noise(&mut self, step_size: f64) -> Vector3 {
        if self.hammer_noise_amplitude <= 0.0 || self.hammer_noise_seconds_remaining <= 0.0 {
            return Vector3::ZERO;
        }
        self.hammer_noise_seconds_remaining -= step_size;
        let elapsed_seconds = HAMMER_NOISE_WINDOW_SECONDS - self.hammer_noise_seconds_remaining;
        let decay_envelope = (-(elapsed_seconds / HAMMER_NOISE_DECAY_SECONDS)).exp();

        let random_sample = next_noise_sample(&mut self.hammer_noise_random_state);

        let filter_coefficient = 1.0 - (-2.0 * PI * self.hammer_noise_cutoff_hz * step_size).exp();
        self.hammer_noise_filter_state +=
            filter_coefficient * (random_sample - self.hammer_noise_filter_state);

        Vector3::new(
            0.0,
            0.0,
            self.hammer_noise_amplitude * self.hammer_noise_filter_state * decay_envelope,
        )
    }

    /// Advance the key release (damper landing) noise burst, returning its
    /// force on the bridge.
    #[inline]
    fn advance_release_noise(&mut self, step_size: f64) -> Vector3 {
        if self.release_noise_seconds_remaining <= 0.0 || self.release_noise_amplitude <= 0.0 {
            return Vector3::ZERO;
        }
        self.release_noise_seconds_remaining -= step_size;
        let elapsed_seconds =
            KEY_RELEASE_NOISE_WINDOW_SECONDS - self.release_noise_seconds_remaining.max(0.0);
        let decay_envelope = (-(elapsed_seconds / KEY_RELEASE_NOISE_DECAY_SECONDS)).exp();

        let random_sample = next_noise_sample(&mut self.release_noise_random_state);

        let filter_coefficient = 1.0 - (-2.0 * PI * KEY_RELEASE_NOISE_CUTOFF_HZ * step_size).exp();
        self.release_noise_filter_state +=
            filter_coefficient * (random_sample - self.release_noise_filter_state);

        Vector3::new(
            0.0,
            0.0,
            self.release_noise_amplitude * self.release_noise_filter_state * decay_envelope,
        )
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
