use super::modal::ModalOscillator;
use crate::engine::math::vector::Vector3;
use std::f64::consts::PI;

pub struct SoundboardDesign {
    pub mode_count: usize,
    pub lowest_mode_frequency: f64,
    pub highest_mode_frequency: f64,
    pub random_seed: u64,
}

pub struct BridgeState {
    pub displacement: Vector3,
    pub acceleration: Vector3,
}

pub struct Soundboard {
    pub modes: Vec<ModalOscillator>,
    pub vertical_bridge_coupling: Vec<f64>,
    pub horizontal_bridge_coupling: Vec<f64>,
    pub longitudinal_bridge_coupling: Vec<f64>,
    pub volume_displacement_coupling: Vec<f64>,
    last_applied_modal_forces: Vec<f64>,
}

impl Soundboard {
    pub fn new(design: &SoundboardDesign) -> Self {
        let mut random = fastrand::Rng::with_seed(design.random_seed);
        let mode_count = design.mode_count.max(1);

        let mut modes = Vec::with_capacity(mode_count);
        let mut vertical_bridge_coupling = Vec::with_capacity(mode_count);
        let mut horizontal_bridge_coupling = Vec::with_capacity(mode_count);
        let mut longitudinal_bridge_coupling = Vec::with_capacity(mode_count);
        let mut volume_displacement_coupling = Vec::with_capacity(mode_count);
        let last_applied_modal_forces = vec![0.0; mode_count];

        for mode_index in 0..mode_count {
            // Logarithmically spaced modes: real soundboards have dense low-frequency modes
            // (supporting A0-F2 fundamentals) and sparser high ones. Linear spacing left the
            // entire sub-100 Hz band nearly empty, starving bass notes of board support.
            let log_lowest = design.lowest_mode_frequency.max(1.0).ln();
            let log_highest = design.highest_mode_frequency.max(1.0).ln();
            let log_fraction = (mode_index as f64 + random.f64()) / mode_count as f64;
            let frequency = (log_lowest + (log_highest - log_lowest) * log_fraction).exp();
            let angular_frequency = 2.0 * PI * frequency;
            // A real piano soundboard carries the body of the tone: its low modes
            // ring for ~0.5-1.5 s, with higher modes dying much faster. The old
            // damping (~0.02-0.06) killed a 120 Hz board mode in ~0.2 s, which is
            // a big part of why notes decayed like a plucked instrument instead of
            // a piano.
            let damping_ratio = uniform(&mut random, 0.005, 0.014) * (1.0 + frequency / 4000.0);            let modal_mass = uniform(&mut random, 0.3, 1.2);

            modes.push(ModalOscillator::new(
                angular_frequency,
                damping_ratio,
                modal_mass,
            ));

            let low_frequency_weight = (-frequency / 700.0).exp();
            vertical_bridge_coupling.push(
                random_sign(&mut random) * (0.11 + 0.85 * low_frequency_weight * random.f64()),
            );
            horizontal_bridge_coupling.push(
                random_sign(&mut random) * (0.04 + 0.30 * low_frequency_weight * random.f64()),
            );
            longitudinal_bridge_coupling.push(
                random_sign(&mut random) * (0.01 + 0.05 * low_frequency_weight * random.f64()),
            );
            volume_displacement_coupling.push(
                random_sign(&mut random)
                    * (0.025 + 0.34 * (-frequency / 450.0).exp() * random.f64()),
            );
        }

        Soundboard {
            modes,
            vertical_bridge_coupling,
            horizontal_bridge_coupling,
            longitudinal_bridge_coupling,
            volume_displacement_coupling,
            last_applied_modal_forces,
        }
    }

    #[inline]
    pub fn apply_forces_and_advance(
        &mut self,
        total_bridge_force_from_strings: Vector3,
        air_pressure_at_board: f64,
        step_size: f64,
    ) {
        let force_x = total_bridge_force_from_strings.x;
        let force_y = total_bridge_force_from_strings.y;
        let force_z = total_bridge_force_from_strings.z;

        for mode_index in 0..self.modes.len() {
            let modal_force = self.vertical_bridge_coupling[mode_index] * force_z
                + self.horizontal_bridge_coupling[mode_index] * force_y
                + self.longitudinal_bridge_coupling[mode_index] * force_x
                - self.volume_displacement_coupling[mode_index] * air_pressure_at_board;

            self.last_applied_modal_forces[mode_index] = modal_force;
            self.modes[mode_index].advance(modal_force, step_size);
        }
    }

    pub fn bridge_state(&self) -> BridgeState {
        let mut displacement = Vector3::ZERO;
        let mut acceleration = Vector3::ZERO;

        for mode_index in 0..self.modes.len() {
            let modal_displacement = self.modes[mode_index].displacement;
            let modal_acceleration = self.modes[mode_index]
                .acceleration_for_force(self.last_applied_modal_forces[mode_index]);

            let longitudinal_coupling = self.longitudinal_bridge_coupling[mode_index];
            let horizontal_coupling = self.horizontal_bridge_coupling[mode_index];
            let vertical_coupling = self.vertical_bridge_coupling[mode_index];

            displacement.x += longitudinal_coupling * modal_displacement;
            acceleration.x += longitudinal_coupling * modal_acceleration;

            displacement.y += horizontal_coupling * modal_displacement;
            acceleration.y += horizontal_coupling * modal_acceleration;

            displacement.z += vertical_coupling * modal_displacement;
            acceleration.z += vertical_coupling * modal_acceleration;
        }

        BridgeState {
            displacement,
            acceleration,
        }
    }

    pub fn volume_velocity(&self) -> f64 {
        let mut total_volume_velocity = 0.0;

        for mode_index in 0..self.modes.len() {
            total_volume_velocity +=
                self.volume_displacement_coupling[mode_index] * self.modes[mode_index].velocity;
        }

        total_volume_velocity
    }

    pub fn volume_acceleration(&self) -> f64 {
        let mut total_volume_acceleration = 0.0;

        for mode_index in 0..self.modes.len() {
            let modal_acceleration = self.modes[mode_index]
                .acceleration_for_force(self.last_applied_modal_forces[mode_index]);

            total_volume_acceleration +=
                self.volume_displacement_coupling[mode_index] * modal_acceleration;
        }

        total_volume_acceleration
    }

    pub fn reset(&mut self) {
        for mode in &mut self.modes {
            mode.displacement = 0.0;
            mode.velocity = 0.0;
        }

        self.last_applied_modal_forces.fill(0.0);
    }
}

#[inline]
fn uniform(random: &mut fastrand::Rng, minimum: f64, maximum: f64) -> f64 {
    minimum + (maximum - minimum) * random.f64()
}

#[inline]
fn random_sign(random: &mut fastrand::Rng) -> f64 {
    if random.bool() { 1.0 } else { -1.0 }
}
