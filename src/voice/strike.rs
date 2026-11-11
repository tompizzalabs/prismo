//! Strike and release behavior: hammer impact, key release noise, hammer noise,
//! damper stepping, and strike-phase detection.

use std::f64::consts::PI;

use crate::engine::math::vector::Vector3;

use super::constants::{
    DAMPER_AMPLITUDE_DECAY_LN, HAMMER_NOISE_DECAY_SECONDS, HAMMER_NOISE_WINDOW_SECONDS,
    KEY_RELEASE_NOISE_AMPLITUDE, KEY_RELEASE_NOISE_CUTOFF_HZ, KEY_RELEASE_NOISE_DECAY_SECONDS,
    KEY_RELEASE_NOISE_WINDOW_SECONDS, STRIKE_PHASE_ANGLE_MARGIN,
};
use super::PianoVoice;

impl PianoVoice {
    /// Begin a hammer strike with the given hammer velocity and deterministically derived noise seed.
    pub fn strike(&mut self, hammer_velocity: f64, noise_seed: u64) {
        self.hammer.rotation_angle = self.grazing_contact_angle;
        let compression_rate_per_radian = self
            .hammer
            .compression_rate_per_radian(self.grazing_contact_angle, self.reference_lateral_offset);
        self.hammer.angular_velocity = hammer_velocity / compression_rate_per_radian.max(1.0e-9);
        self.strike_velocity = hammer_velocity;
        self.damper_decay_per_step = 1.0;
        self.hammer_noise_seconds_remaining = HAMMER_NOISE_WINDOW_SECONDS;
        self.hammer_noise_filter_state = 0.0;
        self.hammer_noise_random_state = noise_seed | 1;
        self.note_age_seconds = 0.0;
        self.blooming_envelope = 1.0;
        self.release_noise_seconds_remaining = 0.0;
        self.release_noise_amplitude = 0.0;
        self.release_noise_filter_state = 0.0;
        self.release_noise_random_state = noise_seed ^ 0x5A5A_5A5A_5A5A_5A5A | 1;
    }

    /// Start shutting the dampers over the given release duration.
    pub fn start_release(&mut self, release_seconds: f64, _engine_step_size: f64) {
        let decay_rate = DAMPER_AMPLITUDE_DECAY_LN / release_seconds.max(1.0e-3);
        self.damper_decay_per_step = decay_rate.exp();
    }

    /// Trigger the audible noise of the damper landing on the strings.
    pub fn trigger_key_release_noise(&mut self) {
        let velocity_scale = (self.strike_velocity / 4.0).clamp(0.2, 1.5);
        self.release_noise_seconds_remaining = KEY_RELEASE_NOISE_WINDOW_SECONDS;
        self.release_noise_amplitude = KEY_RELEASE_NOISE_AMPLITUDE * velocity_scale;
        self.release_noise_filter_state = 0.0;
        self.release_noise_random_state = self
            .release_noise_random_state
            .wrapping_add(0x9E37_79B9_7F4A_7C15)
            | 1;
    }

    pub(super) fn generate_release_noise_force(&mut self, step_size: f64) -> Vector3 {
        if self.release_noise_seconds_remaining <= 0.0 || self.release_noise_amplitude <= 0.0 {
            return Vector3::ZERO;
        }
        self.release_noise_seconds_remaining -= step_size;
        let elapsed_seconds =
            KEY_RELEASE_NOISE_WINDOW_SECONDS - self.release_noise_seconds_remaining.max(0.0);
        let decay_envelope = (-(elapsed_seconds / KEY_RELEASE_NOISE_DECAY_SECONDS)).exp();

        let mut random_state = self.release_noise_random_state;
        random_state ^= random_state >> 12;
        random_state ^= random_state << 25;
        random_state ^= random_state >> 27;
        self.release_noise_random_state = random_state;

        let random_sample = 2.0
            * ((random_state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64
                / (1u64 << 53) as f64)
            - 1.0;

        let filter_coefficient = 1.0 - (-2.0 * PI * KEY_RELEASE_NOISE_CUTOFF_HZ * step_size).exp();
        self.release_noise_filter_state +=
            filter_coefficient * (random_sample - self.release_noise_filter_state);

        Vector3::new(
            0.0,
            0.0,
            self.release_noise_amplitude * self.release_noise_filter_state * decay_envelope,
        )
    }

    /// Apply one damper decay step to every string mode.
    pub fn apply_damper_step(&mut self) {
        let decay_factor = self.damper_decay_per_step;
        if decay_factor >= 1.0 - 1.0e-15 {
            return;
        }
        for piano_string in &mut self.strings {
            let all_modes = piano_string
                .vertical_modes
                .iter_mut()
                .chain(piano_string.horizontal_modes.iter_mut())
                .chain(piano_string.longitudinal_modes.iter_mut());
            for mode in all_modes {
                mode.displacement *= decay_factor;
                mode.velocity *= decay_factor;
            }
        }
    }

    pub(super) fn generate_hammer_noise_force(&mut self, step_size: f64) -> Vector3 {
        if self.hammer_noise_amplitude <= 0.0 || self.hammer_noise_seconds_remaining <= 0.0 {
            return Vector3::ZERO;
        }
        self.hammer_noise_seconds_remaining -= step_size;
        let elapsed_seconds = HAMMER_NOISE_WINDOW_SECONDS - self.hammer_noise_seconds_remaining;
        let decay_envelope = (-(elapsed_seconds / HAMMER_NOISE_DECAY_SECONDS)).exp();

        let mut random_state = self.hammer_noise_random_state;
        random_state ^= random_state >> 12;
        random_state ^= random_state << 25;
        random_state ^= random_state >> 27;
        self.hammer_noise_random_state = random_state;

        let random_sample = 2.0
            * ((random_state.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 11) as f64
                / (1u64 << 53) as f64)
            - 1.0;

        let filter_coefficient = 1.0 - (-2.0 * PI * self.hammer_noise_cutoff_hz * step_size).exp();
        self.hammer_noise_filter_state +=
            filter_coefficient * (random_sample - self.hammer_noise_filter_state);

        Vector3::new(
            0.0,
            0.0,
            self.hammer_noise_amplitude * self.hammer_noise_filter_state * decay_envelope,
        )
    }

    /// Whether the hammer is currently in its strike phase (moving or in contact with strings).
    pub fn is_in_strike_phase(&self, shared_bridge_displacement: Vector3) -> bool {
        if self.hammer.angular_velocity > 0.0
            && self.hammer.rotation_angle >= self.strike_activation_angle
        {
            return true;
        }
        self.any_string_in_contact(shared_bridge_displacement)
    }

    fn any_string_in_contact(&self, shared_bridge_displacement: Vector3) -> bool {
        if self.hammer.rotation_angle < self.strike_activation_angle - STRIKE_PHASE_ANGLE_MARGIN {
            return false;
        }
        for string_index in 0..self.strings.len() {
            let string_displacement =
                self.strings[string_index].displacement_at_strike_point(shared_bridge_displacement);
            let (_, contact_detected) = self.hammer.felt_compression(
                self.hammer.rotation_angle,
                string_displacement,
                self.strings[string_index].lateral_offset_from_hammer_center,
                self.felt_contact_references[string_index],
            );
            if contact_detected {
                return true;
            }
        }
        false
    }
}
