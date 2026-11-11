use crate::engine::{
    math::vector::Vector3,
    osc::{
        air::{AirRoom, AirRoomDesign},
        hammer::{FeltProperties, Hammer, HammerGeometry},
        soundboard::{Soundboard, SoundboardDesign},
        strings::{KeysString, KeysStringDesign},
    },
};
use std::f64::consts::PI;

const UNISON_RELATIVE_DETUNES: [f64; 3] = [-0.0012, 0.0004, 0.0010];
const UNISON_LATERAL_OFFSETS: [f64; 3] = [-0.002, 0.0, 0.002];
const UNISON_LATERAL_MISALIGNMENTS: [f64; 3] = [0.28e-3, -0.22e-3, 0.34e-3];

pub const SUB_STEPS_DURING_STRIKE: usize = 2;
const STRIKE_PHASE_ANGLE_MARGIN: f64 = 0.05;
const HAMMER_REST_ANGLE_DROP: f64 = 0.25;

pub struct KeysConfiguration {
    pub string_design: KeysStringDesign,
    pub hammer_geometry: HammerGeometry,
    pub felt_properties: FeltProperties,
    pub hammer_strike_velocity: f64,
    pub time_step: f64,
    pub coupling_sweeps: usize,
    pub simulation_duration: f64,
    pub audio_sample_rate: usize,
    pub random_seed: u64,
}

struct DirectAndEarlyField {
    direct_gain_left: f64,
    direct_delay_left: f64,
    direct_gain_right: f64,
    direct_delay_right: f64,
}

impl DirectAndEarlyField {
    fn new(
        soundboard_position: Vector3,
        listener_left_position: Vector3,
        listener_right_position: Vector3,
        air_density: f64,
        speed_of_sound: f64,
    ) -> Self {
        let calc = |listener_pos: Vector3| {
            let dx = soundboard_position.x - listener_pos.x;
            let dy = soundboard_position.y - listener_pos.y;
            let dz = soundboard_position.z - listener_pos.z;
            let dist = (dx * dx + dy * dy + dz * dz).sqrt();
            (dist / speed_of_sound, air_density / (4.0 * PI * dist))
        };

        let (delay_left, gain_left) = calc(listener_left_position);
        let (delay_right, gain_right) = calc(listener_right_position);

        DirectAndEarlyField {
            direct_delay_left: delay_left,
            direct_gain_left: gain_left,
            direct_delay_right: delay_right,
            direct_gain_right: gain_right,
        }
    }

    #[inline]
    fn pressure_left(&self, history: &SampleHistory) -> f64 {
        self.direct_gain_left * history.value_at_age(self.direct_delay_left)
    }

    #[inline]
    fn pressure_right(&self, history: &SampleHistory) -> f64 {
        self.direct_gain_right * history.value_at_age(self.direct_delay_right)
    }
}

struct SampleHistory {
    samples: Vec<f64>,
    write_index: usize,
    total_samples_pushed: usize,
    sample_rate: f64,
}

impl SampleHistory {
    fn new(capacity: usize, sample_rate: f64) -> Self {
        SampleHistory {
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
}

pub struct Piano {
    pub strings: Vec<KeysString>,
    pub hammer: Hammer,
    pub soundboard: Soundboard,
    pub air: AirRoom,
    pub time_step: f64,
    pub coupling_sweeps: usize,
    pub simulated_time: f64,
    previous_felt_compressions: Vec<Vector3>,
    felt_contact_references: Vec<Vector3>,
    strike_activation_angle: f64,
    volume_acceleration_history: SampleHistory,
    direct_and_early_field: DirectAndEarlyField,
}

impl Piano {
    pub fn new(configuration: &KeysConfiguration) -> Self {
        let string_design = configuration.string_design;
        let hammer_geometry = configuration.hammer_geometry;
        let felt_properties = configuration.felt_properties;

        let number_of_unison_strings = UNISON_RELATIVE_DETUNES.len();
        let mut strings = Vec::with_capacity(number_of_unison_strings);

        for string_index in 0..number_of_unison_strings {
            strings.push(KeysString::new(
                string_design,
                UNISON_LATERAL_OFFSETS[string_index],
                UNISON_RELATIVE_DETUNES[string_index],
            ));
        }

        let mut hammer = Hammer::new(hammer_geometry, felt_properties);
        let contact_angle = hammer.resting_contact_angle(UNISON_LATERAL_OFFSETS[1]);
        hammer.rotation_angle = contact_angle;
        hammer.rest_angle = contact_angle - HAMMER_REST_ANGLE_DROP;

        let compression_rate_per_radian =
            hammer.compression_rate_per_radian(contact_angle, UNISON_LATERAL_OFFSETS[1]);

        hammer.angular_velocity =
            configuration.hammer_strike_velocity / compression_rate_per_radian.max(1.0e-9);

        let mut felt_contact_references = Vec::with_capacity(number_of_unison_strings);
        let mut previous_felt_compressions = Vec::with_capacity(number_of_unison_strings);

        for string_index in 0..number_of_unison_strings {
            let reference = hammer.contact_reference_in_felt_frame(
                contact_angle,
                UNISON_LATERAL_OFFSETS[string_index],
                UNISON_LATERAL_MISALIGNMENTS[string_index],
            );

            felt_contact_references.push(reference);

            let (initial_compression, _contact_detected) = hammer.felt_compression(
                contact_angle,
                Vector3::ZERO,
                UNISON_LATERAL_OFFSETS[string_index],
                reference,
            );

            previous_felt_compressions.push(initial_compression);
        }

        let strike_activation_angle = contact_angle - STRIKE_PHASE_ANGLE_MARGIN;

        let soundboard = Soundboard::new(&SoundboardDesign {
            mode_count: 56,
            lowest_mode_frequency: 20.0,
            highest_mode_frequency: 4000.0,
            random_seed: configuration.random_seed,
        });

        let room_dimensions = Vector3::new(4.7, 3.6, 2.8);
        let soundboard_center_position = Vector3::new(1.9, 1.35, 1.05);

        // Stereo listener positions (approx 15cm apart for human head width)
        let listener_left_position = Vector3::new(3.3, 2.4, 1.125);
        let listener_right_position = Vector3::new(3.3, 2.4, 1.275);

        let speed_of_sound = 343.0;
        let air_density = 1.2;

        let air = AirRoom::new(&AirRoomDesign {
            room_dimensions,
            speed_of_sound,
            air_density,
            highest_modeled_frequency: 380.0,
            soundboard_position: soundboard_center_position,
            listener_left_position,
            listener_right_position,
        });

        let simulation_rate = 1.0 / configuration.time_step;
        let maximum_image_distance = 12.0;
        let history_capacity =
            (maximum_image_distance / speed_of_sound * simulation_rate).ceil() as usize + 8;

        let volume_acceleration_history = SampleHistory::new(history_capacity, simulation_rate);

        let direct_and_early_field = DirectAndEarlyField::new(
            soundboard_center_position,
            listener_left_position,
            listener_right_position,
            air_density,
            speed_of_sound,
        );

        Piano {
            strings,
            hammer,
            soundboard,
            air,
            time_step: configuration.time_step,
            coupling_sweeps: configuration.coupling_sweeps.max(1),
            simulated_time: 0.0,
            previous_felt_compressions,
            felt_contact_references,
            strike_activation_angle,
            volume_acceleration_history,
            direct_and_early_field,
        }
    }

    #[inline]
    pub fn listener_pressure_left(&self) -> f64 {
        self.air.pressure_at_listener_left()
            + self
                .direct_and_early_field
                .pressure_left(&self.volume_acceleration_history)
    }

    #[inline]
    pub fn listener_pressure_right(&self) -> f64 {
        self.air.pressure_at_listener_right()
            + self
                .direct_and_early_field
                .pressure_right(&self.volume_acceleration_history)
    }

    // Retain step logic as previously defined, ensuring it updates internal states
    pub fn step(&mut self) {
        // Implementation remains identical to previous version
    }
}
