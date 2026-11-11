use super::modal::ModalOscillator;
use crate::engine::math::vector::Vector3;
use std::f64::consts::PI;

const MAX_MODES: usize = 32;

#[derive(Clone, Copy)]
pub struct DampingCoefficients {
    pub constant_part: f64,
    pub stiffness_proportional_part: f64,
}

#[derive(Clone, Copy)]
pub struct KeysStringDesign {
    pub speaking_length: f64,
    pub core_radius: f64,
    /// Effective radius used for bending stiffness (wound strings: larger than core).
    /// Mass and axial stiffness still use `core_radius`.
    pub bending_radius: f64,
    pub material_density: f64,
    pub youngs_modulus: f64,
    pub static_tension: f64,
    pub hammer_strike_ratio: f64,
    pub highest_modeled_frequency: f64,
    pub maximum_transverse_mode_count: usize,
    pub vertical_damping: DampingCoefficients,
    pub horizontal_damping: DampingCoefficients,
    pub longitudinal_damping: DampingCoefficients,
}

#[inline]
fn transverse_mode_angular_frequency(
    wavenumber: f64,
    tuned_tension: f64,
    linear_mass_density: f64,
    bending_stiffness: f64,
) -> f64 {
    let tension_term = (tuned_tension / linear_mass_density) * wavenumber * wavenumber;
    let stiffness_term = (bending_stiffness / linear_mass_density)
        * wavenumber
        * wavenumber
        * wavenumber
        * wavenumber;
    (tension_term + stiffness_term).sqrt()
}

pub struct KeysString {
    pub design: KeysStringDesign,
    pub lateral_offset_from_hammer_center: f64,
    pub linear_mass_density: f64,
    pub bending_stiffness: f64,
    pub axial_stiffness: f64,
    pub tuned_tension: f64,
    pub inverse_speaking_length: f64,
    pub vertical_modes: Vec<ModalOscillator>,
    pub horizontal_modes: Vec<ModalOscillator>,
    pub longitudinal_modes: Vec<ModalOscillator>,
    pub mode_shape_at_strike_point: Vec<f64>,
    pub mode_wavenumbers: Vec<f64>,
    pub alternating_signs: Vec<f64>,
    pub bridge_inertial_coefficients: Vec<f64>,
    pub end_slope_coefficients: Vec<f64>,
}

impl KeysString {
    #[inline]
    pub fn new(design: KeysStringDesign, lateral_offset: f64, relative_detune: f64) -> Self {
        let radius_squared = design.core_radius * design.core_radius;
        let bending_radius_squared = design.bending_radius * design.bending_radius;
        let linear_mass_density = design.material_density * PI * radius_squared;
        let bending_stiffness =
            design.youngs_modulus * PI * bending_radius_squared * bending_radius_squared * 0.25;
        let axial_stiffness = design.youngs_modulus * PI * radius_squared;

        let detune_factor = 1.0 + relative_detune;
        let tuned_tension = design.static_tension * detune_factor * detune_factor;
        let modal_mass = linear_mass_density * design.speaking_length * 0.5;
        let cutoff_angular_frequency = 2.0 * PI * design.highest_modeled_frequency;

        let max_modes = design.maximum_transverse_mode_count.min(MAX_MODES);

        let mut number_of_transverse_modes = 0usize;
        for mode_index in 1..=max_modes {
            let wavenumber = mode_index as f64 * PI / design.speaking_length;
            let angular_frequency = transverse_mode_angular_frequency(
                wavenumber,
                tuned_tension,
                linear_mass_density,
                bending_stiffness,
            );

            if angular_frequency > cutoff_angular_frequency {
                break;
            }

            number_of_transverse_modes = mode_index;
        }

        number_of_transverse_modes = number_of_transverse_modes.max(1);

        let mut vertical_modes = Vec::with_capacity(number_of_transverse_modes);
        let mut horizontal_modes = Vec::with_capacity(number_of_transverse_modes);
        let mut mode_shape_at_strike_point = Vec::with_capacity(number_of_transverse_modes);
        let mut mode_wavenumbers = Vec::with_capacity(number_of_transverse_modes);
        let mut alternating_signs = Vec::with_capacity(number_of_transverse_modes);
        let mut bridge_inertial_coefficients = Vec::with_capacity(number_of_transverse_modes);
        let mut end_slope_coefficients = Vec::with_capacity(number_of_transverse_modes);

        for mode_index in 1..=number_of_transverse_modes {
            let wavenumber = mode_index as f64 * PI / design.speaking_length;
            let angular_frequency = transverse_mode_angular_frequency(
                wavenumber,
                tuned_tension,
                linear_mass_density,
                bending_stiffness,
            );

            let vertical_damping_ratio = (design.vertical_damping.constant_part
                + design.vertical_damping.stiffness_proportional_part * angular_frequency)
                .clamp(1.0e-6, 0.3);

            let horizontal_damping_ratio = (design.horizontal_damping.constant_part
                + design.horizontal_damping.stiffness_proportional_part * angular_frequency)
                .clamp(1.0e-6, 0.3);

            vertical_modes.push(ModalOscillator::new(
                angular_frequency,
                vertical_damping_ratio,
                modal_mass,
            ));

            horizontal_modes.push(ModalOscillator::new(
                angular_frequency,
                horizontal_damping_ratio,
                modal_mass,
            ));

            let alternating_sign = if mode_index % 2 == 0 { 1.0 } else { -1.0 };

            mode_shape_at_strike_point
                .push((mode_index as f64 * PI * design.hammer_strike_ratio).sin());
            mode_wavenumbers.push(wavenumber);
            alternating_signs.push(alternating_sign);
            bridge_inertial_coefficients.push(linear_mass_density * alternating_sign / wavenumber);
            end_slope_coefficients.push(wavenumber * alternating_sign);
        }

        // Longitudinal wave speed is determined by the steel core, not the effective density of the wound string
        let longitudinal_wave_speed = (design.youngs_modulus / 7850.0).sqrt();

        let mut longitudinal_modes = Vec::with_capacity(number_of_transverse_modes);

        for mode_index in 1..=number_of_transverse_modes {
            let wavenumber = mode_index as f64 * PI / design.speaking_length;
            let angular_frequency = wavenumber * longitudinal_wave_speed;

            if angular_frequency > cutoff_angular_frequency {
                break;
            }

            let longitudinal_damping_ratio = (design.longitudinal_damping.constant_part
                + design.longitudinal_damping.stiffness_proportional_part * angular_frequency)
                .clamp(1.0e-6, 0.9);

            longitudinal_modes.push(ModalOscillator::new(
                angular_frequency,
                longitudinal_damping_ratio,
                modal_mass,
            ));
        }

        KeysString {
            design,
            lateral_offset_from_hammer_center: lateral_offset,
            linear_mass_density,
            bending_stiffness,
            axial_stiffness,
            tuned_tension,
            inverse_speaking_length: 1.0 / design.speaking_length,
            vertical_modes,
            horizontal_modes,
            longitudinal_modes,
            mode_shape_at_strike_point,
            mode_wavenumbers,
            alternating_signs,
            bridge_inertial_coefficients,
            end_slope_coefficients,
        }
    }

    #[inline]
    pub fn displacement_at_strike_point(&self, bridge_displacement: Vector3) -> Vector3 {
        let mut displacement = bridge_displacement.scaled(self.design.hammer_strike_ratio);

        for mode_index in 0..self.vertical_modes.len() {
            let shape_at_strike = self.mode_shape_at_strike_point[mode_index];
            displacement.z += self.vertical_modes[mode_index].displacement * shape_at_strike;
            displacement.y += self.horizontal_modes[mode_index].displacement * shape_at_strike;
        }

        for mode_index in 0..self.longitudinal_modes.len() {
            let shape_at_strike = self.mode_shape_at_strike_point[mode_index];
            displacement.x += self.longitudinal_modes[mode_index].displacement * shape_at_strike;
        }

        displacement
    }

    #[inline]
    pub fn apply_forces_and_advance(
        &mut self,
        hammer_force_at_strike_point: Vector3,
        bridge_acceleration: Vector3,
        step_size: f64,
    ) {
        let vertical_force = hammer_force_at_strike_point.z;
        let horizontal_force = hammer_force_at_strike_point.y;
        let longitudinal_force = hammer_force_at_strike_point.x;

        let bridge_vertical_acceleration = bridge_acceleration.z;
        let bridge_horizontal_acceleration = bridge_acceleration.y;
        let bridge_longitudinal_acceleration = bridge_acceleration.x;

        for mode_index in 0..self.vertical_modes.len() {
            let shape_at_strike = self.mode_shape_at_strike_point[mode_index];
            let bridge_coefficient = self.bridge_inertial_coefficients[mode_index];

            let vertical_modal_force = vertical_force * shape_at_strike
                + bridge_coefficient * bridge_vertical_acceleration;

            let horizontal_modal_force = horizontal_force * shape_at_strike
                + bridge_coefficient * bridge_horizontal_acceleration;

            self.vertical_modes[mode_index].advance(vertical_modal_force, step_size);
            self.horizontal_modes[mode_index].advance(horizontal_modal_force, step_size);
        }

        for mode_index in 0..self.longitudinal_modes.len() {
            let shape_at_strike = self.mode_shape_at_strike_point[mode_index];
            let bridge_coefficient = self.bridge_inertial_coefficients[mode_index];

            let longitudinal_modal_force = longitudinal_force * shape_at_strike
                + bridge_coefficient * bridge_longitudinal_acceleration;

            self.longitudinal_modes[mode_index].advance(longitudinal_modal_force, step_size);
        }
    }

    #[inline]
    pub fn force_exerted_on_bridge(&self, bridge_displacement: Vector3) -> Vector3 {
        let mut vertical_end_slope = bridge_displacement.z * self.inverse_speaking_length;
        let mut horizontal_end_slope = bridge_displacement.y * self.inverse_speaking_length;
        let mut longitudinal_end_slope = bridge_displacement.x * self.inverse_speaking_length;

        for mode_index in 0..self.vertical_modes.len() {
            let slope_coefficient = self.end_slope_coefficients[mode_index];

            vertical_end_slope += slope_coefficient * self.vertical_modes[mode_index].displacement;

            horizontal_end_slope +=
                slope_coefficient * self.horizontal_modes[mode_index].displacement;
        }

        for mode_index in 0..self.longitudinal_modes.len() {
            let slope_coefficient = self.end_slope_coefficients[mode_index];

            longitudinal_end_slope +=
                slope_coefficient * self.longitudinal_modes[mode_index].displacement;
        }

        Vector3::new(
            -self.axial_stiffness * longitudinal_end_slope,
            -self.tuned_tension * horizontal_end_slope,
            -self.tuned_tension * vertical_end_slope,
        )
    }
}
