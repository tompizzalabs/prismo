//! Voice construction, energy accounting, silence detection, and re-voicing:
//! adopting the string state of a previously released voice.

use std::f64::consts::PI;

use crate::engine::math::vector::Vector3;
use crate::engine::osc::hammer::{Hammer, HammerGeometry};
use crate::engine::osc::modal::ModalOscillator;
use crate::engine::osc::strings::KeysString;
use crate::instrument::NoteDesign;

use super::constants::{
    DUPLEX_FREQUENCY_RATIOS, DUPLEX_MODAL_DAMPING, DUPLEX_MODAL_MASS, DUPLEX_OUTPUT_SCALE,
    HAMMER_REST_ANGLE_DROP, MAXIMUM_UNISON_STRINGS, STRIKE_PHASE_ANGLE_MARGIN,
    UNISON_LATERAL_MISALIGNMENTS,
};
use super::PianoVoice;

impl PianoVoice {
    /// Build a new voice from a note design and the shared hammer geometry.
    pub fn new(design: &NoteDesign, shared_hammer_geometry: HammerGeometry) -> Self {
        let string_count = design
            .number_of_unison_strings
            .min(MAXIMUM_UNISON_STRINGS)
            .max(1);

        let mut strings = Vec::with_capacity(string_count);
        for string_index in 0..string_count {
            strings.push(KeysString::new(
                design.string,
                design.unison_lateral_offsets[string_index],
                design.unison_relative_detunes[string_index],
            ));
        }

        let mut hammer = Hammer::new(shared_hammer_geometry, design.felt);
        let reference_lateral_offset = design.unison_lateral_offsets[(string_count - 1) / 2];
        let contact_angle = hammer.resting_contact_angle(reference_lateral_offset);
        hammer.rest_angle = contact_angle - HAMMER_REST_ANGLE_DROP;
        hammer.rotation_angle = contact_angle;
        hammer.angular_velocity = 0.0;

        let mut felt_contact_references = [Vector3::ZERO; MAXIMUM_UNISON_STRINGS];
        let mut previous_felt_compressions = [Vector3::ZERO; MAXIMUM_UNISON_STRINGS];
        for string_index in 0..string_count {
            let reference = hammer.contact_reference_in_felt_frame(
                contact_angle,
                design.unison_lateral_offsets[string_index],
                UNISON_LATERAL_MISALIGNMENTS[string_index]
                    + design.voicing.lateral_misalignment_shift,
            );
            felt_contact_references[string_index] = reference;

            let (initial_compression, _contact_detected) = hammer.felt_compression(
                contact_angle,
                Vector3::ZERO,
                design.unison_lateral_offsets[string_index],
                reference,
            );
            previous_felt_compressions[string_index] = initial_compression;
        }

        let mut duplex_resonators = Vec::with_capacity(DUPLEX_FREQUENCY_RATIOS.len());
        let mut duplex_reaction_coefficients = Vec::with_capacity(DUPLEX_FREQUENCY_RATIOS.len());
        for &frequency_ratio in &DUPLEX_FREQUENCY_RATIOS {
            let resonant_frequency = (design.fundamental_frequency * frequency_ratio).min(20_000.0);
            let angular_frequency = 2.0 * PI * resonant_frequency;
            duplex_resonators.push(ModalOscillator::new(
                angular_frequency,
                DUPLEX_MODAL_DAMPING,
                DUPLEX_MODAL_MASS,
            ));
            duplex_reaction_coefficients.push(
                DUPLEX_MODAL_MASS * angular_frequency * angular_frequency * DUPLEX_OUTPUT_SCALE,
            );
        }

        let snapshot_capacity: usize = strings
            .iter()
            .map(|piano_string| {
                (piano_string.vertical_modes.len()
                    + piano_string.horizontal_modes.len()
                    + piano_string.longitudinal_modes.len())
                    * 2
            })
            .sum::<usize>()
            + 2;

        let mut hammer_force_scales = [1.0; MAXIMUM_UNISON_STRINGS];
        for string_index in 0..string_count {
            hammer_force_scales[string_index] = design.voicing.hammer_force_scales[string_index];
        }

        PianoVoice {
            strike_velocity: 0.0,
            release_noise_seconds_remaining: 0.0,
            release_noise_amplitude: 0.0,
            release_noise_filter_state: 0.0,
            release_noise_random_state: 0x2545_F491_4F6C_DD15,
            strings,
            hammer,
            previous_felt_compressions,
            felt_contact_references,
            grazing_contact_angle: contact_angle,
            strike_activation_angle: contact_angle - STRIKE_PHASE_ANGLE_MARGIN,
            reference_lateral_offset,
            hammer_force_scales,
            damper_decay_per_step: 1.0,
            hammer_noise_seconds_remaining: 0.0,
            hammer_noise_amplitude: design.voicing.hammer_noise_amplitude,
            hammer_noise_cutoff_hz: design.voicing.hammer_noise_cutoff_hz,
            hammer_noise_filter_state: 0.0,
            hammer_noise_random_state: 1,
            note_age_seconds: 0.0,
            blooming_envelope: 1.0,
            bridge_coupling_factor: 1.0,
            duplex_output_scale: 1.0,
            duplex_resonators,
            duplex_reaction_coefficients,
            snapshot_buffer: Vec::with_capacity(snapshot_capacity),
        }
    }

    /// Total mechanical energy currently stored in the unison string modes.
    pub fn string_energy(&self) -> f64 {
        let mut total_energy = 0.0;
        for piano_string in &self.strings {
            let all_modes = piano_string
                .vertical_modes
                .iter()
                .chain(piano_string.horizontal_modes.iter())
                .chain(piano_string.longitudinal_modes.iter());
            for mode in all_modes {
                total_energy += 0.5
                    * mode.modal_mass
                    * (mode.velocity * mode.velocity
                        + mode.angular_frequency
                            * mode.angular_frequency
                            * mode.displacement
                            * mode.displacement);
            }
        }
        total_energy
    }

    /// Whether the voice can be considered finished (no energy and no active noises).
    pub fn is_silent(&self, energy_threshold: f64) -> bool {
        self.string_energy() < energy_threshold
            && self.release_noise_seconds_remaining <= 0.0
            && self.hammer_noise_seconds_remaining <= 0.0
    }

    /// Carry over the string (and duplex resonator) vibration state from another voice,
    /// which is used when re-voicing the same note for seamless legato.
    pub fn adopt_string_state_from(&mut self, other: &PianoVoice) {
        for (destination_string, source_string) in self.strings.iter_mut().zip(other.strings.iter())
        {
            for (destination, source) in destination_string
                .vertical_modes
                .iter_mut()
                .zip(source_string.vertical_modes.iter())
            {
                destination.displacement = source.displacement;
                destination.velocity = source.velocity;
            }
            for (destination, source) in destination_string
                .horizontal_modes
                .iter_mut()
                .zip(source_string.horizontal_modes.iter())
            {
                destination.displacement = source.displacement;
                destination.velocity = source.velocity;
            }
            for (destination, source) in destination_string
                .longitudinal_modes
                .iter_mut()
                .zip(source_string.longitudinal_modes.iter())
            {
                destination.displacement = source.displacement;
                destination.velocity = source.velocity;
            }
        }

        for (destination, source) in self
            .duplex_resonators
            .iter_mut()
            .zip(other.duplex_resonators.iter())
        {
            destination.displacement = source.displacement;
            destination.velocity = source.velocity;
        }

        for string_index in 0..self.strings.len() {
            let strike_point_displacement =
                self.strings[string_index].displacement_at_strike_point(Vector3::ZERO);
            let (compression, _) = self.hammer.felt_compression(
                self.grazing_contact_angle,
                strike_point_displacement,
                self.strings[string_index].lateral_offset_from_hammer_center,
                self.felt_contact_references[string_index],
            );
            self.previous_felt_compressions[string_index] = compression;
        }
    }
}
