use crate::engine::math::vector::Vector3;
use crate::engine::osc::air::{AirRoom, AirRoomDesign, EarlyField};
use crate::engine::osc::hammer::{FeltProperties, HammerGeometry};
use crate::engine::osc::soundboard::{Soundboard, SoundboardDesign};
use crate::engine::osc::strings::{DampingCoefficients, KeysStringDesign};
use crate::voice::{SUB_STEPS_DURING_STRIKE, SynthVoice};
use std::f64::consts::PI;

pub const SIMULATION_RATE_HZ: f64 = 48_000.0;
pub const SIMULATION_STEP_SIZE: f64 = 1.0 / SIMULATION_RATE_HZ;
pub const COUPLING_SWEEPS: usize = 1;
const RINGOUT_SECONDS: f64 = 0.75;

#[inline]
fn linear_interpolation(low: f64, high: f64, fraction: f64) -> f64 {
    low + (high - low) * fraction
}

#[derive(Clone, Copy)]
pub struct StrikeVoicingControls {
    pub hammer_hardness: f64,
    pub hammer_hardness_piano: f64,
    pub hammer_hardness_mezzo: f64,
    pub hammer_hardness_forte: f64,
    pub hammer_noise: f64,
    pub hammer_tone: f64,
    pub soft_pedal: f64,
    pub unison_width: f64,
    pub string_length_scale: f64,
    pub strike_point_ratio: f64,
}

#[derive(Clone, Copy)]
pub struct LiveVoicingControls {
    pub sympathetic_resonance: f64,
    pub duplex_scale: f64,
    pub blooming_energy: f64,
    pub blooming_inertia: f64,
}

impl Default for LiveVoicingControls {
    fn default() -> Self {
        LiveVoicingControls {
            sympathetic_resonance: 1.0,
            duplex_scale: 1.0,
            blooming_energy: 0.5,
            blooming_inertia: 1.0,
        }
    }
}

#[derive(Clone, Copy)]
pub struct VoicingBaked {
    pub hammer_force_scales: [f64; 3],
    pub lateral_misalignment_shift: f64,
    pub hammer_noise_amplitude: f64,
    pub hammer_noise_cutoff_hz: f64,
}

#[derive(Clone, Copy)]
pub struct NoteDesign {
    pub string: KeysStringDesign,
    pub felt: FeltProperties,
    pub number_of_unison_strings: usize,
    pub unison_relative_detunes: [f64; 3],
    pub unison_lateral_offsets: [f64; 3],
    pub fundamental_frequency: f64,
    pub voicing: VoicingBaked,
}

fn velocity_hardness_curve(
    velocity_blend: f64,
    hardness_piano: f64,
    hardness_mezzo: f64,
    hardness_forte: f64,
) -> f64 {
    if velocity_blend < 0.5 {
        linear_interpolation(hardness_piano, hardness_mezzo, velocity_blend * 2.0)
    } else {
        linear_interpolation(hardness_mezzo, hardness_forte, (velocity_blend - 0.5) * 2.0)
    }
}

fn railsback_stretch_cents(note: usize) -> f64 {
    let normalized = ((note as f64 - 60.0) / 48.0).clamp(-1.0, 1.0);
    12.0 * normalized + 4.0 * normalized * normalized * normalized
}

pub fn note_design(
    midi_note: u8,
    controls: &StrikeVoicingControls,
    velocity_blend: f64,
) -> NoteDesign {
    let note = midi_note.clamp(21, 108) as usize;
    let note_fraction = (note - 21) as f64 / 87.0;
    let nominal_fundamental_frequency = 440.0 * ((note as f64 - 69.0) / 12.0).exp2();

    let stretch_factor = 2.0_f64.powf(railsback_stretch_cents(note) / 1200.0);
    let fundamental_frequency = nominal_fundamental_frequency * stretch_factor;

    let length_scale = controls.string_length_scale.clamp(0.8, 10.0);
    let base_speaking_length = if note >= 60 {
        0.62 * (0.052_f64 / 0.62).powf((note - 60) as f64 / 48.0)
    } else {
        (0.62 * (261.6256 / nominal_fundamental_frequency).powf(0.90)).min(2.05)
    };
    let speaking_length = base_speaking_length * length_scale;

    let strike_ratio = controls.strike_point_ratio.clamp(1.0 / 64.0, 0.5);

    let core_radius = if note <= 45 {
        linear_interpolation(0.35e-3, 0.60e-3, (note - 21) as f64 / 24.0)
    } else if note <= 60 {
        linear_interpolation(0.60e-3, 0.475e-3, (note - 45) as f64 / 15.0)
    } else {
        linear_interpolation(0.475e-3, 0.33e-3, (note - 60) as f64 / 48.0)
    };

    let tension = linear_interpolation(750.0, 550.0, note_fraction) * length_scale * length_scale;
    // Preserve the exact linear mass density required to hit the stretched fundamental.
    // The plain-steel floor is removed: bass/mid strings are modeled as wound strings whose
    // effective (average) density is legitimately lower than solid steel. Flooring it at 7850
    // distorted the produced frequency by up to ~100 cents around notes 43-53.
    let linear_mass_density = tension / (2.0 * speaking_length * fundamental_frequency).powi(2);
    let effective_density =
        (linear_mass_density / (PI * core_radius * core_radius)).max(1500.0);

    // Wound strings have a much larger bending radius than their core: the copper wrap adds
    // bending stiffness without contributing to the core's axial stiffness. This restores the
    // strong bass inharmonicity (beta ~ 1e-3 low down) that real pianos exhibit, tapering to a
    // plain string above ~note 50.
    let winding_amount = (1.0 - (note as f64 - 21.0) / 29.0).clamp(0.0, 1.0);
    let winding_thickness = winding_amount.powf(1.2) * 0.8e-3;
    let bending_radius = core_radius + winding_thickness;

    let highest_modeled_frequency = (24.0 * fundamental_frequency)
        .min(8000.0)
        .max(4.5 * fundamental_frequency);

    let string = KeysStringDesign {
        speaking_length,
        core_radius,
        bending_radius,
        material_density: effective_density,
        youngs_modulus: 2.0e11,
        static_tension: tension,
        hammer_strike_ratio: strike_ratio,
        highest_modeled_frequency,
        maximum_transverse_mode_count: 48,
        vertical_damping: DampingCoefficients {
            constant_part: 2.2e-4,
            stiffness_proportional_part: 5.5e-8,
        },
        horizontal_damping: DampingCoefficients {
            constant_part: 0.9e-4,
            stiffness_proportional_part: 2.5e-8,
        },
        longitudinal_damping: DampingCoefficients {
            constant_part: 3.0e-3,
            stiffness_proportional_part: 0.0,
        },
    };

    let velocity_hardness = velocity_hardness_curve(
        velocity_blend,
        controls.hammer_hardness_piano,
        controls.hammer_hardness_mezzo,
        controls.hammer_hardness_forte,
    );
    let effective_hardness = (controls.hammer_hardness + velocity_hardness - 1.0).clamp(-3.0, 3.0);
    let stiffness_multiplier = 2.0_f64.powf(effective_hardness * 1.5);
    let base_felt_stiffness =
        linear_interpolation(9.0e10, 4.5e11, note_fraction) * stiffness_multiplier;

    let felt_exponent = (linear_interpolation(2.7, 3.4, note_fraction)
        + effective_hardness * 0.15
        + controls.hammer_tone * 0.25)
        .clamp(1.8, 4.2);

    let felt_relaxation_time = (linear_interpolation(3.2e-4, 1.2e-4, note_fraction)
        * 2.0_f64.powf(-effective_hardness * 0.5))
    .max(3.0e-5);

    let felt = FeltProperties {
        stiffness: [
            base_felt_stiffness / 6.0,
            base_felt_stiffness / 6.0,
            base_felt_stiffness,
        ],
        nonlinearity_exponent: [felt_exponent; 3],
        relaxation_time: [felt_relaxation_time; 3],
    };

    let soft_pedal = controls.soft_pedal.clamp(0.0, 1.0);
    let hammer_force_scales = [1.0 - soft_pedal, 1.0, 1.0];
    let lateral_misalignment_shift = soft_pedal * 1.5e-3;

    let hammer_noise_amplitude =
        controls.hammer_noise.clamp(0.0, 3.0) * (0.8 + 3.0 * velocity_blend.clamp(0.0, 1.0));
    let hammer_noise_cutoff_hz =
        1500.0 * 30.0_f64.powf((controls.hammer_tone.clamp(-1.0, 1.0) + 1.0) * 0.5);

    let unison_width = controls.unison_width.clamp(0.0, 20.0);
    // Bass fundamentals beat too slowly to read as a unison when the detune is the same in cents,
    // so spread the low/mid strings further apart. Ramp of ~3x at note 21 down to 1x by note 45.
    let low_register_unison_boost =
        1.0 + ((45.0 - note as f64) / 24.0).clamp(0.0, 1.0) * 2.0;
    let unison_relative_detunes = [-1.2, 0.4, 1.0]
        .map(|detune| detune * unison_width * 1.0e-4 * low_register_unison_boost);

    let number_of_unison_strings = if note < 28 {
        1
    } else if note < 41 {
        2
    } else {
        3
    };

    NoteDesign {
        string,
        felt,
        number_of_unison_strings,
        unison_relative_detunes,
        unison_lateral_offsets: [-0.002, 0.0, 0.002],
        fundamental_frequency,
        voicing: VoicingBaked {
            hammer_force_scales,
            lateral_misalignment_shift,
            hammer_noise_amplitude,
            hammer_noise_cutoff_hz,
        },
    }
}

pub fn shared_hammer_geometry() -> HammerGeometry {
    HammerGeometry {
        horizontal_arm_length: 0.135,
        vertical_arm_length: 0.020,
        felt_thickness: 0.012,
        string_distance_from_pivot: 0.152,
        string_angle_from_pivot: 23.0_f64.to_radians(),
        string_inclination_angle: 0.0,
        shank_line_density: 0.185,
        pivot_damping: 2.0e-4,
    }
}

pub struct Instrument {
    pub soundboard: Soundboard,
    pub air: AirRoom,
    pub ear_field: EarlyField,
    pub live_voicing: LiveVoicingControls,
    ringout_steps_remaining: u64,
    pub simulated_time: f64,
}

impl Instrument {
    pub fn new(random_seed: u64) -> Self {
        let room_dimensions = Vector3::new(4.7, 3.6, 2.8);
        let soundboard_center_position = Vector3::new(1.9, 1.35, 1.05);
        let listener_left_position = Vector3::new(3.3, 2.4, 1.125);
        let listener_right_position = Vector3::new(3.3, 2.4, 1.275);
        let speed_of_sound = 343.0;
        let air_density = 1.2;

        let air_design = AirRoomDesign {
            room_dimensions,
            speed_of_sound,
            air_density,
            highest_modeled_frequency: 300.0,
            soundboard_position: soundboard_center_position,
            listener_left_position,
            listener_right_position,
        };

        let soundboard = Soundboard::new(&SoundboardDesign {
            mode_count: 56,
            lowest_mode_frequency: 20.0,
            highest_mode_frequency: 4000.0,
            random_seed,
        });

        let air = AirRoom::new(&air_design);
        let ear_field = EarlyField::new(&air_design, SIMULATION_RATE_HZ);

        Instrument {
            soundboard,
            air,
            ear_field,
            live_voicing: LiveVoicingControls::default(),
            ringout_steps_remaining: 0,
            simulated_time: 0.0,
        }
    }

    pub fn step(&mut self, voices: &mut [Option<SynthVoice>]) -> (f64, f64) {
        let any_voice_active = voices.iter().any(|voice| voice.is_some());
        if !any_voice_active && self.ringout_steps_remaining == 0 {
            self.simulated_time += SIMULATION_STEP_SIZE;
            return (0.0, 0.0);
        }

        self.ringout_steps_remaining = if any_voice_active {
            (RINGOUT_SECONDS * SIMULATION_RATE_HZ) as u64
        } else {
            self.ringout_steps_remaining.saturating_sub(1)
        };

        let strike_phase_is_active = if any_voice_active {
            let bridge_state_for_phase_check = self.soundboard.bridge_state();
            voices.iter().flatten().any(|voice| {
                voice
                    .engine
                    .is_in_strike_phase(bridge_state_for_phase_check.displacement)
            })
        } else {
            false
        };

        let sub_step_count = if strike_phase_is_active {
            SUB_STEPS_DURING_STRIKE
        } else {
            1
        };

        let sub_step_size = SIMULATION_STEP_SIZE / sub_step_count as f64;
        let live_voicing = self.live_voicing;

        for voice in voices.iter_mut().flatten() {
            if voice.releasing {
                voice.engine.apply_damper_step();
            }
            voice.engine.update_slow_modulations(
                SIMULATION_STEP_SIZE,
                live_voicing.blooming_energy,
                live_voicing.blooming_inertia,
                live_voicing.sympathetic_resonance,
                live_voicing.duplex_scale,
            );
        }

        for _ in 0..sub_step_count {
            let shared_bridge_state = self.soundboard.bridge_state();
            let mut total_bridge_force = Vector3::ZERO;

            for voice in voices.iter_mut().flatten() {
                total_bridge_force = total_bridge_force
                    + voice
                        .engine
                        .advance(sub_step_size, &shared_bridge_state, COUPLING_SWEEPS);
            }

            let air_pressure_at_soundboard = self.air.pressure_at_soundboard();
            self.soundboard.apply_forces_and_advance(
                total_bridge_force,
                air_pressure_at_soundboard,
                sub_step_size,
            );

            let soundboard_volume_velocity = self.soundboard.volume_velocity();
            self.air.advance(soundboard_volume_velocity, sub_step_size);
        }

        // Feed the soundboard's volume acceleration into the per-ear direct and
        // early-reflection field, so each ear gets its own ITD/ILD taps.
        let volume_acceleration = self.soundboard.volume_acceleration();
        self.ear_field.push(volume_acceleration);

        self.simulated_time += SIMULATION_STEP_SIZE;

        (
            self.air.pressure_at_listener_left() + self.ear_field.pressure_left(),
            self.air.pressure_at_listener_right() + self.ear_field.pressure_right(),
        )
    }

    pub fn reset(&mut self) {
        self.soundboard.reset();
        self.air.reset();
        self.ear_field.reset();
        self.ringout_steps_remaining = 0;
        self.simulated_time = 0.0;
        self.live_voicing = LiveVoicingControls::default();
    }
}

#[cfg(test)]
mod output_level_tests {
    use super::*;
    use crate::engine::osc::hammer::HammerGeometry;
    use crate::master::OutputMaster;
    use crate::voice::PianoVoice;
    use nice_plug::midi::{Channel, Key, VoiceID};

    struct LevelReport {
        peak: f64,
        mastered_peak: f64,
        rms: f64,
        room_peak: f64,
        early_peak: f64,
        label: String,
    }

    fn strike_controls(strike_randomness: &mut fastrand::Rng) -> StrikeVoicingControls {
        StrikeVoicingControls {
            hammer_hardness: 0.0,
            hammer_hardness_piano: 1.0,
            hammer_hardness_mezzo: 1.0,
            hammer_hardness_forte: 1.0,
            hammer_noise: 0.9 + (1.1 - 0.9) * strike_randomness.f64(),
            hammer_tone: 0.0,
            soft_pedal: 0.0,
            unison_width: 9.5 + (10.5 - 9.5) * strike_randomness.f64(),
            string_length_scale: 1.0,
            strike_point_ratio: 0.122,
        }
    }

    fn make_voice(
        note: u8,
        velocity: f32,
        voice_id: u64,
        geometry: HammerGeometry,
    ) -> SynthVoice {
        let hammer_velocity = (0.5 + (4.0 - 0.5) * velocity as f64).clamp(0.05, 12.0);
        let velocity_blend = ((hammer_velocity - 0.5) / (4.0 - 0.5)).clamp(0.0, 1.0);
        let noise_seed = voice_id.wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ ((note as u64) << 32);
        let mut randomness = fastrand::Rng::with_seed(noise_seed);
        let design = note_design(note, &strike_controls(&mut randomness), velocity_blend);
        let mut engine_voice = PianoVoice::new(&design, geometry);
        engine_voice.strike(hammer_velocity, noise_seed);
        SynthVoice {
            voice_id: VoiceID::ID(voice_id as i32),
            internal_voice_id: voice_id,
            channel: Channel::Number(0),
            key: Key::Number(note),
            note,
            releasing: false,
            engine: engine_voice,
        }
    }

    fn measure(label: &str, voices: &mut [Option<SynthVoice>; 8]) -> LevelReport {
        let mut instrument = Instrument::new(20240903);
        let mut master = OutputMaster::new(SIMULATION_RATE_HZ);
        let mut peak = 0.0f64;
        let mut mastered_peak = 0.0f64;
        let mut room_peak = 0.0f64;
        let mut early_peak = 0.0f64;
        let mut sum_squares = 0.0f64;
        let mut count = 0u64;

        for _ in 0..(SIMULATION_RATE_HZ as u64 * 3) {
            let (left, right) = instrument.step(voices);
            let (mastered_left, mastered_right) = master.process(left, right);
            let amplitude = left.abs().max(right.abs());
            if amplitude > peak {
                peak = amplitude;
            }
            let mastered_amplitude = mastered_left.abs().max(mastered_right.abs());
            if mastered_amplitude > mastered_peak {
                mastered_peak = mastered_amplitude;
            }
            let room_amplitude = instrument
                .air
                .pressure_at_listener_left()
                .abs()
                .max(instrument.air.pressure_at_listener_right().abs());
            if room_amplitude > room_peak {
                room_peak = room_amplitude;
            }
            let early_amplitude = instrument
                .ear_field
                .pressure_left()
                .abs()
                .max(instrument.ear_field.pressure_right().abs());
            if early_amplitude > early_peak {
                early_peak = early_amplitude;
            }
            sum_squares += left * left + right * right;
            count += 1;
        }

        let rms = (sum_squares / (2.0 * count as f64)).sqrt();
        LevelReport {
            peak,
            mastered_peak,
            rms,
            room_peak,
            early_peak,
            label: label.to_string(),
        }
    }

    #[test]
    fn print_output_levels() {
        println!("\n=== OUTPUT LEVEL MEASUREMENTS (pre-master, 0 dB gain) ===");
        println!("{:<36} {:>12} {:>12} {:>10}", "scenario", "peak", "peak dB", "rms dB");

        let geometry = shared_hammer_geometry();

        let scenarios: Vec<(&str, Vec<(u8, f32)>)> = vec![
            ("A4 mezzo (vel 0.25)", vec![(69, 0.25)]),
            ("A4 loud (vel 0.5)", vec![(69, 0.5)]),
            ("A4 fortissimo (vel 1.0)", vec![(69, 1.0)]),
            ("C6 fortissimo (vel 1.0)", vec![(84, 1.0)]),
            ("C2 fortissimo (vel 1.0)", vec![(36, 1.0)]),
            (
                "8-note chord fff + sustain",
                vec![
                    (36, 1.0),
                    (43, 1.0),
                    (48, 1.0),
                    (55, 1.0),
                    (60, 1.0),
                    (64, 1.0),
                    (67, 1.0),
                    (72, 1.0),
                ],
            ),
        ];

        for (label, notes) in scenarios {
            let mut voices: [Option<SynthVoice>; 8] = [0; 8].map(|_| None);
            for (index, &(note, velocity)) in notes.iter().enumerate() {
                voices[index] = Some(make_voice(note, velocity, index as u64, geometry));
            }
            let report = measure(label, &mut voices);
            let peak_db = 20.0 * report.peak.max(1.0e-12).log10();
            let mastered_db = 20.0 * report.mastered_peak.max(1.0e-12).log10();
            let rms_db = 20.0 * report.rms.max(1.0e-12).log10();
            let room_db = 20.0 * report.room_peak.max(1.0e-12).log10();
            let early_db = 20.0 * report.early_peak.max(1.0e-12).log10();
            println!(
                "{:<36} {:>9.1} dB  mastered {:>8.1} dB  rms {:>8.1} dB  room {:>7.1}  early {:>7.1}",
                report.label, peak_db, mastered_db, rms_db, room_db, early_db
            );
            let ceiling = 10.0_f64.powf(crate::master::LIMITER_CEILING_DB / 20.0);
            assert!(
                report.mastered_peak <= ceiling + 1.0e-6,
                "{} exceeded master ceiling: {}",
                report.label,
                report.mastered_peak
            );
        }
    }

    // Measure the envelope of a single note: windowed RMS across its life, in dB
    // relative to the loudest window. This shows the attack and decay shape.
    fn measure_envelope(label: &str, note: u8, velocity: f32) {
        let geometry = shared_hammer_geometry();
        let mut voices: [Option<SynthVoice>; 8] = [0; 8].map(|_| None);
        voices[0] = Some(make_voice(note, velocity, 0, geometry));
        let mut instrument = Instrument::new(20240903);

        // Window boundaries in seconds (relative to note start).
        let windows: &[f64] = &[0.0, 0.02, 0.1, 0.4, 1.0, 2.0, 3.5, 5.5];
        let mut rms: Vec<f64> = vec![0.0; windows.len() - 1];
        let mut counts: Vec<u64> = vec![0; windows.len() - 1];
        let mut total_seconds = 0.0f64;
        let duration = 5.5;

        let dt = SIMULATION_STEP_SIZE;
        while total_seconds < duration {
            let (left, right) = instrument.step(&mut voices);
            // Absolute (full-scale) sample magnitude; unaffected by limiter.
            let magnitude = (left * left + right * right).sqrt();
            let mut window = windows.len() - 2;
            for (index, &boundary) in windows.iter().enumerate().skip(1) {
                if total_seconds < boundary {
                    window = index - 1;
                    break;
                }
            }
            rms[window] += magnitude * magnitude;
            counts[window] += 1;
            total_seconds += dt;
        }

        let mut peak_db: Vec<f64> = Vec::with_capacity(rms.len());
        for (index, sum) in rms.iter().enumerate() {
            let value = (sum / counts[index].max(1) as f64).sqrt();
            peak_db.push(20.0 * value.max(1.0e-12).log10());
        }
        let reference = peak_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
        println!("\n{} (note {}, vel {}): envelope windows (dB rel. to loudest window):", label, note, velocity);
        for index in 0..peak_db.len() {
            let start = windows[index];
            let end = windows[index + 1];
            println!("  {:>5.2}-{:<5.2}s: {:>7.1} dB", start, end, peak_db[index] - reference);
        }
    }

    #[test]
    fn print_envelope() {
        println!("\n=== ENVELOPE / DECAY SHAPE ===");
        println!("(This is the pre-master signal, shapes unaffected by output trim/limiter.)");
        measure_envelope("A4 fortissimo", 69, 1.0);
        measure_envelope("C2 fortissimo", 36, 1.0);
        measure_envelope("C6 fortissimo", 84, 1.0);
        measure_envelope("C6 piano", 84, 0.2);
        measure_envelope("A0 long", 21, 0.8);
    }

    // Simple one-pole low/high-pass helpers for band analysis.
    #[inline]
    fn one_pole_lp(input: f64, state: &mut f64, coefficient: f64) -> f64 {
        *state += coefficient * (input - *state);
        *state
    }

    // Per-frequency-band envelope: uses single-pole filters to split the output
    // into low (<400 Hz), mid (400-1600 Hz) and high (>1600 Hz) bands, then
    // reports each band's windowed RMS relative to its own loudest window. This
    // exposes the frequency-dependent decay that separates a piano (long warm
    // low sustain) from a harp/ukulele (bright, short).
    fn measure_spectral_envelope(label: &str, note: u8, velocity: f32) {
        let geometry = shared_hammer_geometry();
        let mut voices: [Option<SynthVoice>; 8] = [0; 8].map(|_| None);
        voices[0] = Some(make_voice(note, velocity, 0, geometry));
        let mut instrument = Instrument::new(20240903);

        let windows: &[f64] = &[0.0, 0.03, 0.2, 0.7, 1.5, 3.0, 5.0];
        let mut lows: Vec<f64> = vec![0.0; windows.len() - 1];
        let mut mids: Vec<f64> = vec![0.0; windows.len() - 1];
        let mut highs: Vec<f64> = vec![0.0; windows.len() - 1];
        let mut counts: Vec<u64> = vec![0; windows.len() - 1];

        let dt = SIMULATION_STEP_SIZE;
        let low_a = 1.0 - (-2.0 * PI * 400.0 * dt).exp();
        let mid_a = 1.0 - (-2.0 * PI * 1600.0 * dt).exp();

        let mut lp400 = 0.0f64;
        let mut lp1600 = 0.0f64;
        let mut total_seconds = 0.0f64;
        while total_seconds < 5.0 {
            let (left, right) = instrument.step(&mut voices);
            let sample = (left + right) * 0.5;
            lp400 = one_pole_lp(sample, &mut lp400, low_a);
            lp1600 = one_pole_lp(sample, &mut lp1600, mid_a);
            // Band-pass approximations.
            let low_band = lp400;
            let mid_band = lp1600 - lp400;
            let high_band = sample - lp1600;

            let mut window = windows.len() - 2;
            for (index, &boundary) in windows.iter().enumerate().skip(1) {
                if total_seconds < boundary {
                    window = index - 1;
                    break;
                }
            }
            lows[window] += low_band * low_band;
            mids[window] += mid_band * mid_band;
            highs[window] += high_band * high_band;
            counts[window] += 1;
            total_seconds += dt;
        }

        let to_db = |band: &[f64]| -> Vec<f64> {
            band.iter()
                .enumerate()
                .map(|(i, sum)| 20.0 * ((sum / counts[i].max(1) as f64).sqrt().max(1.0e-12)).log10())
                .collect()
        };
        let low_db = to_db(&lows);
        let mid_db = to_db(&mids);
        let high_db = to_db(&highs);
        let rel = |band_db: &[f64]| -> Vec<f64> {
            let ref_peak = band_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            band_db.iter().map(|v| v - ref_peak).collect()
        };

        println!("\n{} (note {}, vel {}): band envelopes, dB rel. to each band's loudest window [low <400Hz | mid 400-1600 | high >1600Hz]:", label, note, velocity);
        let low_rel = rel(&low_db);
        let mid_rel = rel(&mid_db);
        let high_rel = rel(&high_db);
        for i in 0..low_rel.len() {
            let start = windows[i];
            let end = windows[i + 1];
            println!(
                "  {:>5.2}-{:<5.2}s:  L {:>7.1}  M {:>7.1}  H {:>7.1}",
                start, end, low_rel[i], mid_rel[i], high_rel[i]
            );
        }
    }

    #[test]
    fn print_spectral_envelope() {
        println!("\n=== SPECTRAL ENVELOPE (per-band decay) ===");
        measure_spectral_envelope("A4 fortissimo", 69, 1.0);
        measure_spectral_envelope("C2 fortissimo", 36, 1.0);
        measure_spectral_envelope("C4 mezzo", 60, 0.5);
        measure_spectral_envelope("C6 fortissimo", 84, 1.0);
    }

    // Measure the actual string partial amplitudes over time (per vertical mode),
    // the physical ground truth of string sustain. Reports each partial's
    // amplitude envelope in dB relative to its own peak.
    fn measure_string_partials(label: &str, note: u8, velocity: f32) {
        let geometry = shared_hammer_geometry();
        let mut voices: [Option<SynthVoice>; 8] = [0; 8].map(|_| None);
        voices[0] = Some(make_voice(note, velocity, 0, geometry));
        let mut instrument = Instrument::new(20240903);

        // Windows in seconds.
        let windows: &[f64] = &[0.0, 0.1, 0.5, 1.0, 2.0, 4.0];
        let partial_count = voices[0].as_ref().unwrap().engine.strings[0]
            .vertical_modes
            .len()
            .min(12);
        let mut energy: Vec<Vec<f64>> = vec![vec![0.0; windows.len() - 1]; partial_count];
        let mut counts: Vec<u64> = vec![0; windows.len() - 1];
        let mut total_seconds = 0.0f64;

        while total_seconds < 4.0 {
            instrument.step(&mut voices);
            // Take the first (primary) unison string's vertical mode displacements.
            let string = &voices[0].as_ref().unwrap().engine.strings[0];
            let mut window = windows.len() - 2;
            for (index, &boundary) in windows.iter().enumerate().skip(1) {
                if total_seconds < boundary {
                    window = index - 1;
                    break;
                }
            }
            for partial in 0..partial_count {
                let displacement = string.vertical_modes[partial].displacement;
                energy[partial][window] += displacement * displacement;
            }
            counts[window] += 1;
            total_seconds += SIMULATION_STEP_SIZE;
        }

        println!("\n{} (note {}, vel {}): string vertical partial T60-ish [dB rel. to each partial's peak]:", label, note, velocity);
        for partial in 0..partial_count {
            let mut band_db: Vec<f64> = energy[partial]
                .iter()
                .enumerate()
                .map(|(i, sum)| 20.0 * ((sum / counts[i].max(1) as f64).sqrt().max(1.0e-12)).log10())
                .collect();
            let reference = band_db.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
            for value in &mut band_db {
                *value -= reference;
            }
            if (partial + 1) == 1 || (partial + 1) <= 3 || (partial + 1) % 2 == 0 {
                print!("  p{:>2}:", partial + 1);
                for value in &band_db {
                    print!(" {:>7.1}", value);
                }
                println!();
            }
        }
    }

    #[test]
    fn print_string_partials() {
        println!("\n=== STRING PARTIAL SUSTAIN ===");
        measure_string_partials("A4 fortissimo", 69, 1.0);
        measure_string_partials("C2 fortissimo", 36, 1.0);
    }
}
