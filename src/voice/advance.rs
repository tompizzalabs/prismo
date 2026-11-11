//! Per-step simulation: the hammer/string/bridge coupling sweep loop, slow
//! parameter modulations, and state snapshotting used for multi-sweep coupling.

use crate::engine::math::vector::Vector3;
use crate::engine::osc::soundboard::BridgeState;

use super::constants::{DUPLEX_DRIVE_SCALE, MAXIMUM_UNISON_STRINGS};
use super::PianoVoice;

impl PianoVoice {
    /// Advance the voice by one engine step, returning the force exerted on the bridge.
    pub fn advance(
        &mut self,
        step_size: f64,
        shared_bridge_state: &BridgeState,
        coupling_sweeps: usize,
    ) -> Vector3 {
        let string_count = self.strings.len();
        let sweep_count = coupling_sweeps.max(1);
        let hammer_angle_at_step_start = self.hammer.rotation_angle;

        let mut felt_compression_at_step_start = [Vector3::ZERO; MAXIMUM_UNISON_STRINGS];
        for string_index in 0..string_count {
            let string_displacement_at_strike = self.strings[string_index]
                .displacement_at_strike_point(shared_bridge_state.displacement);
            let (compression, _contact_detected) = self.hammer.felt_compression(
                hammer_angle_at_step_start,
                string_displacement_at_strike,
                self.strings[string_index].lateral_offset_from_hammer_center,
                self.felt_contact_references[string_index],
            );
            felt_compression_at_step_start[string_index] = compression;
        }

        let noise_forces = self.noise.advance(step_size);

        let mut state_snapshot = Vec::new();
        if sweep_count > 1 {
            state_snapshot = std::mem::take(&mut self.snapshot_buffer);
            state_snapshot.clear();
            self.write_state_snapshot(&mut state_snapshot);
        }

        let scaled_bridge_acceleration = shared_bridge_state
            .acceleration
            .scaled(self.bridge_coupling_factor);

        let mut hammer_angle_source = hammer_angle_at_step_start;
        let mut felt_compression_source = felt_compression_at_step_start;
        let mut compression_reference_for_rate_term = self.previous_felt_compressions;
        let mut bridge_force = Vector3::ZERO;
        let last_sweep_index = sweep_count - 1;

        for sweep_index in 0..sweep_count {
            if sweep_index > 0 {
                self.restore_state_snapshot(&state_snapshot);
                compression_reference_for_rate_term = felt_compression_at_step_start;
            }

            let mut total_felt_force_on_strings = Vector3::ZERO;
            let mut forces_on_strings = [Vector3::ZERO; MAXIMUM_UNISON_STRINGS];
            for string_index in 0..string_count {
                let mut felt_frame_force = self.hammer.felt_contact_force(
                    felt_compression_source[string_index],
                    compression_reference_for_rate_term[string_index],
                    step_size,
                );
                felt_frame_force = felt_frame_force * self.hammer_force_scales[string_index];
                total_felt_force_on_strings = total_felt_force_on_strings + felt_frame_force;
                forces_on_strings[string_index] = self
                    .hammer
                    .force_on_string_in_string_frame(felt_frame_force, hammer_angle_source)
                    + noise_forces.hammer_on_string;
            }

            for string_index in 0..string_count {
                self.strings[string_index].apply_forces_and_advance(
                    forces_on_strings[string_index],
                    scaled_bridge_acceleration,
                    step_size,
                );
            }

            let total_shank_torque = self
                .hammer
                .shank_torque(hammer_angle_source, total_felt_force_on_strings)
                + self.hammer.rest_rail_torque();
            self.hammer
                .advance_shank_rotation(total_shank_torque, step_size);

            let mut voice_bridge_force = Vector3::ZERO;
            for piano_string in &self.strings {
                voice_bridge_force = voice_bridge_force
                    + piano_string.force_exerted_on_bridge(shared_bridge_state.displacement);
            }

            if sweep_index == last_sweep_index && self.duplex_output_scale > 1.0e-6 {
                let duplex_drive =
                    voice_bridge_force.z * self.duplex_output_scale * DUPLEX_DRIVE_SCALE;
                for resonator in &mut self.duplex_resonators {
                    resonator.advance(duplex_drive, step_size);
                }
                let mut duplex_reaction_force = 0.0;
                for (resonator, &reaction_coefficient) in self
                    .duplex_resonators
                    .iter()
                    .zip(self.duplex_reaction_coefficients.iter())
                {
                    duplex_reaction_force += reaction_coefficient * resonator.displacement;
                }
                voice_bridge_force.z += duplex_reaction_force;
            }

            bridge_force = voice_bridge_force * self.blooming_envelope;

            if sweep_index == last_sweep_index {
                bridge_force =
                    bridge_force + noise_forces.hammer_on_bridge + noise_forces.release_on_bridge;
            }

            hammer_angle_source = self.hammer.rotation_angle;

            let mut predicted_felt_compressions = [Vector3::ZERO; MAXIMUM_UNISON_STRINGS];
            for string_index in 0..string_count {
                let string_displacement_at_strike = self.strings[string_index]
                    .displacement_at_strike_point(shared_bridge_state.displacement);
                let (compression, _) = self.hammer.felt_compression(
                    hammer_angle_source,
                    string_displacement_at_strike,
                    self.strings[string_index].lateral_offset_from_hammer_center,
                    self.felt_contact_references[string_index],
                );
                predicted_felt_compressions[string_index] = compression;
            }
            felt_compression_source = predicted_felt_compressions;
        }

        if sweep_count > 1 {
            self.snapshot_buffer = state_snapshot;
        }

        self.previous_felt_compressions = felt_compression_at_step_start;
        bridge_force
    }

    /// Update the slowly-varying modulation envelopes (bloom, bridge coupling, duplex output).
    pub fn update_slow_modulations(
        &mut self,
        step_size: f64,
        blooming_energy: f64,
        blooming_inertia: f64,
        sympathetic_resonance: f64,
        duplex_scale: f64,
    ) {
        self.note_age_seconds += step_size;
        self.bridge_coupling_factor = sympathetic_resonance;
        self.duplex_output_scale = duplex_scale;

        if blooming_energy > 1.0e-4 && self.note_age_seconds < 30.0 * blooming_inertia {
            let inverse_inertia = 1.0 / blooming_inertia.max(1.0e-6);
            let age = self.note_age_seconds;
            let rise = 1.0 - (-age * inverse_inertia).exp();
            let fall = (-age * inverse_inertia / 6.0).exp();
            self.blooming_envelope = 1.0 + blooming_energy * rise * fall;
        } else {
            self.blooming_envelope = 1.0;
        }
    }

    fn write_state_snapshot(&self, snapshot: &mut Vec<f64>) {
        for piano_string in &self.strings {
            let all_modes = piano_string
                .vertical_modes
                .iter()
                .chain(piano_string.horizontal_modes.iter())
                .chain(piano_string.longitudinal_modes.iter());
            for mode in all_modes {
                snapshot.push(mode.displacement);
                snapshot.push(mode.velocity);
            }
        }
        snapshot.push(self.hammer.rotation_angle);
        snapshot.push(self.hammer.angular_velocity);
    }

    fn restore_state_snapshot(&mut self, snapshot: &[f64]) {
        let mut cursor = 0usize;
        for piano_string in &mut self.strings {
            let all_modes = piano_string
                .vertical_modes
                .iter_mut()
                .chain(piano_string.horizontal_modes.iter_mut())
                .chain(piano_string.longitudinal_modes.iter_mut());
            for mode in all_modes {
                mode.displacement = snapshot[cursor];
                mode.velocity = snapshot[cursor + 1];
                cursor += 2;
            }
        }
        self.hammer.rotation_angle = snapshot[cursor];
        self.hammer.angular_velocity = snapshot[cursor + 1];
    }
}
