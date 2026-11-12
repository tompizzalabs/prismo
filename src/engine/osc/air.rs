//! Sound radiation in the air: a 3-D room modal field plus a per-ear
//! direct + early reflection field.
//!
//! This mirrors the radiation model of the reference paper (arXiv:2409.03481):
//! the room is a shoebox whose pressure field is expanded into standing-wave
//! (modal) components, and the direct sound is modeled as a point-monopole
//! field with wall image sources. The listeners sit at two separate positions
//! (left/right ear), so each ear receives genuinely different mode shapes,
//! inter-aural time differences (ITD) and level differences (ILD) — making the
//! output naturally wide and 3-D instead of mono.

use crate::engine::{
    math::{complex::Complex64, vector::Vector3},
    osc::modal::ComplexModalMode,
};
use std::f64::consts::PI;

/// Number of lowest-frequency 3-D room modes retained in the modal field.
const TARGET_ROOM_MODE_COUNT: usize = 96;
/// Reflection gain applied to each first-order wall image source.
const WALL_REFLECTION_COEFFICIENT: f64 = 0.55;
/// Overall scale of the direct + early reflection field relative to the modal field.
///
/// The direct-field taps carry `rho / (4 * pi * distance)` while the modal field is
/// normalized by `rho*c^2 / V`. These live on very different scales for the room and
/// source geometry used here; this constant empirically re-balances the direct/early
/// field so it sits alongside (rather than dominating) the room modal tail, keeping the
/// 3-D image audible without blowing out the master.
const DIRECT_FIELD_SCALE: f64 = 0.08;

pub struct AirRoomDesign {
    pub room_dimensions: Vector3,
    pub speed_of_sound: f64,
    pub air_density: f64,
    pub highest_modeled_frequency: f64,
    pub soundboard_position: Vector3,
    pub listener_left_position: Vector3,
    pub listener_right_position: Vector3,
}

pub struct AirRoom {
    pub modal_states: Vec<ComplexModalMode>,
    source_gain_per_volume_velocity: Vec<Complex64>,
    soundboard_pressure_coefficients: Vec<f64>,
    listener_pressure_coefficients_left: Vec<f64>,
    listener_pressure_coefficients_right: Vec<f64>,
}

/// Pressure mode shape of a rigid-wall shoebox room: a separable product of
/// cosines, one per axis. Using all three axes makes the two listener positions
/// (which differ in position) genuinely different, unlike the old 1-D model.
#[inline]
fn room_mode_shape(
    position: Vector3,
    room_dimensions: Vector3,
    mode_indices: (usize, usize, usize),
) -> f64 {
    let phase_x = mode_indices.0 as f64 * PI * position.x / room_dimensions.x;
    let phase_y = mode_indices.1 as f64 * PI * position.y / room_dimensions.y;
    let phase_z = mode_indices.2 as f64 * PI * position.z / room_dimensions.z;
    phase_x.cos() * phase_y.cos() * phase_z.cos()
}

/// Resonant frequency of a 3-D rectangular-room mode (l, m, n).
#[inline]
fn room_mode_frequency(
    speed_of_sound: f64,
    room_dimensions: Vector3,
    mode_indices: (usize, usize, usize),
) -> f64 {
    let fx = mode_indices.0 as f64 / room_dimensions.x;
    let fy = mode_indices.1 as f64 / room_dimensions.y;
    let fz = mode_indices.2 as f64 / room_dimensions.z;
    0.5 * speed_of_sound * (fx * fx + fy * fy + fz * fz).sqrt()
}

impl AirRoom {
    pub fn new(design: &AirRoomDesign) -> Self {
        let room = design.room_dimensions;
        let cutoff = design.highest_modeled_frequency.clamp(1.0, 1.0e5);

        // Build the mode-selection grid from the modeled-frequency cutoff:
        // a mode is kept when its resonance is below `highest_modeled_frequency`.
        let max_l = (2.0 * cutoff * room.x / design.speed_of_sound).ceil() as usize;
        let max_m = (2.0 * cutoff * room.y / design.speed_of_sound).ceil() as usize;
        let max_n = (2.0 * cutoff * room.z / design.speed_of_sound).ceil() as usize;

        let mut candidates: Vec<(f64, usize, usize, usize)> = Vec::new();
        for index_l in 0..=max_l {
            for index_m in 0..=max_m {
                for index_n in 0..=max_n {
                    if index_l == 0 && index_m == 0 && index_n == 0 {
                        // The (0,0,0) mode is spatially uniform and has zero
                        // resonant frequency; it only couples to the room's
                        // static pressure, so it is skipped.
                        continue;
                    }
                    let frequency = room_mode_frequency(
                        design.speed_of_sound,
                        room,
                        (index_l, index_m, index_n),
                    );
                    if frequency <= cutoff {
                        candidates.push((frequency, index_l, index_m, index_n));
                    }
                }
            }
        }

        candidates.sort_by(|left, right| left.0.total_cmp(&right.0));
        candidates.truncate(TARGET_ROOM_MODE_COUNT);
        if candidates.is_empty() {
            candidates.push((20.0, 1, 1, 0));
        }

        let modal_count = candidates.len();
        let mut modal_states = Vec::with_capacity(modal_count);
        let mut source_gain_per_volume_velocity = Vec::with_capacity(modal_count);
        let mut soundboard_pressure_coefficients = Vec::with_capacity(modal_count);
        let mut listener_pressure_coefficients_left = Vec::with_capacity(modal_count);
        let mut listener_pressure_coefficients_right = Vec::with_capacity(modal_count);

        let density_c2 = design.air_density * design.speed_of_sound * design.speed_of_sound;

        for (rank, &(frequency, index_l, index_m, index_n)) in candidates.iter().enumerate() {
            let angular_frequency = 2.0 * PI * frequency;
            // Higher room modes are absorbed a little more strongly, as real
            // walls do for short wavelengths.
            let damping_ratio = (0.012 + 0.0012 * rank as f64).clamp(1.0e-4, 0.5);

            let shape_at_soundboard = room_mode_shape(
                design.soundboard_position,
                room,
                (index_l, index_m, index_n),
            );
            let shape_at_listener_left = room_mode_shape(
                design.listener_left_position,
                room,
                (index_l, index_m, index_n),
            );
            let shape_at_listener_right = room_mode_shape(
                design.listener_right_position,
                room,
                (index_l, index_m, index_n),
            );

            modal_states.push(ComplexModalMode::new(angular_frequency, damping_ratio));

            soundboard_pressure_coefficients.push(shape_at_soundboard);
            listener_pressure_coefficients_left.push(shape_at_listener_left);
            listener_pressure_coefficients_right.push(shape_at_listener_right);

            let damped_frequency = angular_frequency * (1.0 - damping_ratio * damping_ratio).sqrt();
            let complex_eigenvalue =
                Complex64::new(-damping_ratio * angular_frequency, damped_frequency);

            let projection_denominator = Complex64::new(2.0, 0.0)
                + complex_eigenvalue * (2.0 * damping_ratio / angular_frequency);

            // Modal source normalization: the standard room-mode radiation
            // coefficient is rho*c^2 / V (V = room volume), giving the coupling
            // between a monopole volume-acceleration source at the soundboard
            // and the room's pressure field. Dividing by the room *volume*
            // (rather than a single length) keeps the pressure in a sane
            // physical range.
            let room_volume = room.x * room.y * room.z;
            let pressure_source_scale = density_c2 / room_volume * shape_at_soundboard;

            source_gain_per_volume_velocity.push(
                projection_denominator
                    .inverse()
                    .scaled(pressure_source_scale),
            );
        }

        AirRoom {
            modal_states,
            source_gain_per_volume_velocity,
            soundboard_pressure_coefficients,
            listener_pressure_coefficients_left,
            listener_pressure_coefficients_right,
        }
    }

    #[inline]
    pub fn advance(&mut self, soundboard_volume_velocity: f64, step_size: f64) {
        for (mode, source_gain) in self
            .modal_states
            .iter_mut()
            .zip(self.source_gain_per_volume_velocity.iter())
        {
            mode.advance(source_gain.scaled(soundboard_volume_velocity), step_size);
        }
    }

    #[inline]
    pub fn pressure_at_soundboard(&self) -> f64 {
        let mut pressure = 0.0;
        for (mode, coefficient) in self
            .modal_states
            .iter()
            .zip(self.soundboard_pressure_coefficients.iter())
        {
            pressure += coefficient * mode.state.real_part;
        }
        pressure
    }

    #[inline]
    pub fn pressure_at_listener_left(&self) -> f64 {
        let mut pressure = 0.0;
        for (mode, coefficient) in self
            .modal_states
            .iter()
            .zip(self.listener_pressure_coefficients_left.iter())
        {
            pressure += coefficient * mode.state.real_part;
        }
        pressure
    }

    #[inline]
    pub fn pressure_at_listener_right(&self) -> f64 {
        let mut pressure = 0.0;
        for (mode, coefficient) in self
            .modal_states
            .iter()
            .zip(self.listener_pressure_coefficients_right.iter())
        {
            pressure += coefficient * mode.state.real_part;
        }
        pressure
    }

    pub fn reset(&mut self) {
        for mode in &mut self.modal_states {
            mode.state = Complex64::ZERO;
        }
    }
}

/// A small circular buffer that samples a signal at arbitrary historical ages
/// (with fractional-sample, i.e. linear, interpolation). Used to render the
/// per-ear propagation delays of the direct and early reflected sound.
pub struct DelayLine {
    samples: Vec<f64>,
    write_index: usize,
    total_samples_pushed: usize,
    sample_rate: f64,
}

impl DelayLine {
    fn new(capacity: usize, sample_rate: f64) -> Self {
        DelayLine {
            samples: vec![0.0; capacity.max(4)],
            write_index: 0,
            total_samples_pushed: 0,
            sample_rate,
        }
    }

    #[inline]
    fn push(&mut self, value: f64) {
        self.samples[self.write_index] = value;
        self.write_index = (self.write_index + 1) % self.samples.len();
        self.total_samples_pushed += 1;
    }

    #[inline]
    fn value_at_age(&self, age_seconds: f64) -> f64 {
        if age_seconds < 0.0 || self.total_samples_pushed == 0 {
            return 0.0;
        }

        let fractional_position = age_seconds * self.sample_rate;
        let whole_steps_back = fractional_position.floor() as usize;
        let fraction = fractional_position - whole_steps_back as f64;

        let capacity = self.samples.len();
        let available_history = self.total_samples_pushed.min(capacity);

        if whole_steps_back + 1 >= available_history {
            return 0.0;
        }

        let newest_index = (self.write_index + capacity - 1) % capacity;
        let index_newer = (newest_index + capacity - whole_steps_back) % capacity;
        let index_older = (newest_index + capacity - whole_steps_back - 1) % capacity;

        self.samples[index_newer] * (1.0 - fraction) + self.samples[index_older] * fraction
    }

    fn reset(&mut self) {
        self.samples.fill(0.0);
        self.write_index = 0;
        self.total_samples_pushed = 0;
    }
}

/// Per-ear direct sound plus first-order image-source reflections off the room
/// walls. Each ear accumulates the same monopole source history with its own
/// set of (delay, gain) taps, which naturally produces inter-aural time and
/// level differences (ITD / ILD) — the physical basis of a wide, 3-D image.
pub struct EarlyField {
    history: DelayLine,
    taps_left: Vec<(f64, f64)>,
    taps_right: Vec<(f64, f64)>,
    source_scale: f64,
}

impl EarlyField {
    pub fn new(design: &AirRoomDesign, simulation_rate: f64) -> Self {
        let room = design.room_dimensions;
        let speed_of_sound = design.speed_of_sound;
        let air_density = design.air_density;

        // Worst-case delay is close to the room's space diagonal.
        let diagonal_x = 2.0 * room.x;
        let diagonal_y = 2.0 * room.y;
        let diagonal_z = 2.0 * room.z;
        let room_diagonal =
            (diagonal_x * diagonal_x + diagonal_y * diagonal_y + diagonal_z * diagonal_z).sqrt();
        let history_capacity =
            ((room_diagonal / speed_of_sound) * simulation_rate).ceil() as usize + 8;

        // The real source plus the six first-order mirror images (one per wall).
        let source_position = design.soundboard_position;
        let image_sources: [Vector3; 7] = [
            source_position,
            Vector3::new(-source_position.x, source_position.y, source_position.z),
            Vector3::new(
                2.0 * room.x - source_position.x,
                source_position.y,
                source_position.z,
            ),
            Vector3::new(source_position.x, -source_position.y, source_position.z),
            Vector3::new(
                source_position.x,
                2.0 * room.y - source_position.y,
                source_position.z,
            ),
            Vector3::new(source_position.x, source_position.y, -source_position.z),
            Vector3::new(
                source_position.x,
                source_position.y,
                2.0 * room.z - source_position.z,
            ),
        ];

        let build_taps = |listener: Vector3| -> Vec<(f64, f64)> {
            image_sources
                .iter()
                .enumerate()
                .map(|(source_index, image)| {
                    let dx = image.x - listener.x;
                    let dy = image.y - listener.y;
                    let dz = image.z - listener.z;
                    let dist = (dx * dx + dy * dy + dz * dz).sqrt().max(1.0e-3);
                    let reflection_gain = if source_index == 0 {
                        1.0
                    } else {
                        WALL_REFLECTION_COEFFICIENT
                    };
                    // Monopole radiation: pressure ~ 1/distance, delayed by d/c.
                    (
                        dist / speed_of_sound,
                        air_density / (4.0 * PI * dist) * reflection_gain,
                    )
                })
                .collect()
        };

        EarlyField {
            history: DelayLine::new(history_capacity, simulation_rate),
            taps_left: build_taps(design.listener_left_position),
            taps_right: build_taps(design.listener_right_position),
            source_scale: DIRECT_FIELD_SCALE,
        }
    }

    /// Push one sample of the monopole source signal (the soundboard's volume
    /// acceleration) into the propagation delay line.
    #[inline]
    pub fn push(&mut self, source_signal: f64) {
        self.history.push(source_signal * self.source_scale);
    }

    #[inline]
    pub fn pressure_left(&self) -> f64 {
        let mut pressure = 0.0;
        for &(delay, gain) in &self.taps_left {
            pressure += gain * self.history.value_at_age(delay);
        }
        pressure
    }

    #[inline]
    pub fn pressure_right(&self) -> f64 {
        let mut pressure = 0.0;
        for &(delay, gain) in &self.taps_right {
            pressure += gain * self.history.value_at_age(delay);
        }
        pressure
    }

    pub fn reset(&mut self) {
        self.history.reset();
    }
}
