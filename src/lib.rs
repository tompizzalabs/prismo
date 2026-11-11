mod engine;
mod instrument;
mod params;
mod presets;
mod ui;
mod voice;

pub use params::PrismoParams;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use nice_plug::midi::{Channel, Key, VoiceID};
use nice_plug::prelude::*;
use nice_plug_egui::{EguiEditor, EguiEditorState, RepaintNotifier};

use crate::engine::osc::hammer::HammerGeometry;
use crate::instrument::{
    Instrument, LiveVoicingControls, SIMULATION_RATE_HZ, SIMULATION_STEP_SIZE,
    StrikeVoicingControls, note_design, shared_hammer_geometry,
};
use crate::ui::{EditorSharedState, INITIAL_SCALE_FACTOR, INITIAL_WINDOW_SIZE};
use crate::voice::{PianoVoice, SynthVoice};

const NUM_VOICES: usize = 8;
const MAX_BLOCK_SIZE: usize = 64;
const VOICE_ENERGY_EPSILON: f64 = 1.0e-9;
const PEAK_METER_DECAY_MS: f64 = 150.0;
const PRESSURE_RING_SIZE: u64 = 8;

pub struct Prismo {
    params: Arc<PrismoParams>,
    editor_state: Arc<EguiEditorState>,
    instrument: Instrument,
    shared_hammer_geometry: HammerGeometry,
    voices: [Option<SynthVoice>; NUM_VOICES],
    next_internal_voice_id: u64,
    held_keys: [bool; 128],
    sustain_pedal_down: bool,
    engine_steps_rendered: u64,
    output_samples_rendered: u64,
    engine_steps_per_output_sample: f64,
    pressure_ring_left: Vec<f64>,
    pressure_ring_right: Vec<f64>,
    peak_meter: Arc<AtomicF32>,
    peak_meter_decay_weight: f32,
    active_voice_count: Arc<AtomicU32>,
    repaint_notifier: RepaintNotifier,
    initial_editor: Option<ui::PianoEditor>,
}

impl Default for Prismo {
    fn default() -> Self {
        let params = Arc::new(PrismoParams::default());
        let peak_meter = Arc::new(AtomicF32::new(util::MINUS_INFINITY_DB));
        let active_voice_count = Arc::new(AtomicU32::new(0));

        let shared_state = EditorSharedState {
            params: params.clone(),
            peak_meter: peak_meter.clone(),
            active_voice_count: active_voice_count.clone(),
        };

        let initial_editor = ui::PianoEditor::new(shared_state, INITIAL_SCALE_FACTOR);

        Self {
            params,
            editor_state: EguiEditorState::from_size(INITIAL_WINDOW_SIZE, INITIAL_SCALE_FACTOR),
            instrument: Instrument::new(20240903),
            shared_hammer_geometry: shared_hammer_geometry(),
            voices: [0; NUM_VOICES].map(|_| None),
            next_internal_voice_id: 0,
            held_keys: [false; 128],
            sustain_pedal_down: false,
            engine_steps_rendered: 0,
            output_samples_rendered: 0,
            engine_steps_per_output_sample: 1.0,
            pressure_ring_left: vec![0.0; PRESSURE_RING_SIZE as usize],
            pressure_ring_right: vec![0.0; PRESSURE_RING_SIZE as usize],
            peak_meter,
            peak_meter_decay_weight: 1.0,
            active_voice_count,
            repaint_notifier: RepaintNotifier::new(),
            initial_editor: Some(initial_editor),
        }
    }
}

impl Plugin for Prismo {
    const NAME: &'static str = "Prismo";
    const VENDOR: &'static str = "Tom Pizza";
    const URL: &'static str = "https://example.com/prismo";
    const EMAIL: &'static str = "info@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: None,
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    const MIDI_INPUT: MidiConfig = MidiConfig::Basic;
    const SAMPLE_ACCURATE_AUTOMATION: bool = true;

    type Editor = EguiEditor<ui::PianoEditor>;
    type SysExMessage = ();
    type BackgroundTask = ();

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn editor(&mut self, _async_executor: AsyncExecutor<Self>) -> Option<Self::Editor> {
        ui::build_editor(
            self.editor_state.clone(),
            self.repaint_notifier.clone(),
            self.initial_editor.take().unwrap(),
        )
    }

    fn reset(&mut self) {
        self.instrument.reset();
        for voice_slot in self.voices.iter_mut() {
            *voice_slot = None;
        }
        self.next_internal_voice_id = 0;
        self.held_keys.fill(false);
        self.sustain_pedal_down = false;
        self.engine_steps_rendered = 0;
        self.output_samples_rendered = 0;
        self.pressure_ring_left.fill(0.0);
        self.pressure_ring_right.fill(0.0);
        self.peak_meter
            .store(util::MINUS_INFINITY_DB, Ordering::Relaxed);
    }

    fn activate(
        &mut self,
        _audio_io_layout: &AudioIOLayout,
        buffer_config: &BufferConfig,
        _context: &mut impl ActivateContext<Self>,
    ) -> bool {
        self.engine_steps_per_output_sample = SIMULATION_RATE_HZ / buffer_config.sample_rate as f64;
        self.peak_meter_decay_weight = 0.25f64
            .powf((buffer_config.sample_rate as f64 * PEAK_METER_DECAY_MS / 1000.0).recip())
            as f32;
        true
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        self.instrument.live_voicing = LiveVoicingControls {
            sympathetic_resonance: self.params.sympathetic_resonance.value() as f64,
            duplex_scale: self.params.duplex_scale_resonance.value() as f64,
            blooming_energy: self.params.blooming_energy.value() as f64,
            blooming_inertia: self.params.blooming_inertia.value() as f64,
        };

        let num_samples = buffer.samples();
        let output = buffer.as_slice();
        let mut next_event = context.next_event();

        let mut block_start: usize = 0;
        let mut block_end: usize = MAX_BLOCK_SIZE.min(num_samples);

        while block_start < num_samples {
            'events: loop {
                match next_event {
                    Some(event) if (event.timing() as usize) <= block_start => {
                        match event {
                            NoteEvent::NoteOn {
                                timing,
                                voice_id,
                                channel,
                                key,
                                velocity,
                            } => {
                                self.start_voice(context, timing, voice_id, channel, key, velocity);
                            }
                            NoteEvent::NoteOff {
                                voice_id,
                                channel,
                                key,
                                ..
                            } => {
                                self.start_release_for_voices(voice_id, channel, key);
                            }
                            NoteEvent::Choke {
                                timing,
                                voice_id,
                                channel,
                                key,
                            } => {
                                self.choke_voices(context, timing, voice_id, channel, key);
                            }
                            NoteEvent::MidiCC {
                                timing: _,
                                channel: _,
                                cc,
                                value,
                            } if cc == 64 => {
                                let pedal_down = if value <= 1.0 {
                                    value >= 0.5
                                } else {
                                    value >= 64.0
                                };
                                self.set_sustain_pedal(pedal_down);
                            }
                            _ => (),
                        };
                        next_event = context.next_event();
                    }
                    Some(event) if (event.timing() as usize) < block_end => {
                        block_end = event.timing() as usize;
                        break 'events;
                    }
                    _ => break 'events,
                }
            }

            for sample_index in block_start..block_end {
                let (left_engine, right_engine) = self.render_one_output_sample();
                let gain = self.params.output_gain.smoothed.next();

                let left_sample = left_engine as f32 * gain;
                let right_sample = right_engine as f32 * gain;

                output[0][sample_index] = left_sample;
                output[1][sample_index] = right_sample;

                if self.editor_state.is_open() {
                    let amplitude = left_sample.abs().max(right_sample.abs());
                    let current_peak = self.peak_meter.load(Ordering::Relaxed);
                    let new_peak = if amplitude > current_peak {
                        amplitude
                    } else {
                        current_peak * self.peak_meter_decay_weight
                            + amplitude * (1.0 - self.peak_meter_decay_weight)
                    };
                    let clamped_peak = if new_peak < 0.0001 { 0.0 } else { new_peak };
                    if current_peak != clamped_peak {
                        self.peak_meter.store(clamped_peak, Ordering::Relaxed);
                        self.repaint_notifier.request_repaint();
                    }
                }
            }

            for voice_slot in self.voices.iter_mut() {
                let terminate = match voice_slot {
                    Some(voice) => voice.engine.is_silent(VOICE_ENERGY_EPSILON),
                    None => false,
                };
                if terminate {
                    if let Some(voice) = voice_slot.take() {
                        let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                            timing: block_end as u32,
                            voice_id: voice.voice_id,
                            channel: voice.channel,
                            key: voice.key,
                        });
                    }
                }
            }

            if self.editor_state.is_open() {
                let active_count =
                    self.voices.iter().filter(|voice| voice.is_some()).count() as u32;
                if self.active_voice_count.load(Ordering::Relaxed) != active_count {
                    self.active_voice_count
                        .store(active_count, Ordering::Relaxed);
                    self.repaint_notifier.request_repaint();
                }
            }

            block_start = block_end;
            block_end = (block_start + MAX_BLOCK_SIZE).min(num_samples);
        }

        ProcessStatus::Normal
    }
}

impl Prismo {
    fn render_one_output_sample(&mut self) -> (f64, f64) {
        let engine_position =
            self.output_samples_rendered as f64 * self.engine_steps_per_output_sample;
        let base_step = engine_position.floor() as u64;
        let fraction = engine_position - base_step as f64;

        while self.engine_steps_rendered < base_step + 2 {
            let (left, right) = self.instrument.step(&mut self.voices);
            let ring_index = (self.engine_steps_rendered % PRESSURE_RING_SIZE) as usize;
            self.pressure_ring_left[ring_index] = left;
            self.pressure_ring_right[ring_index] = right;
            self.engine_steps_rendered += 1;
        }

        let idx = (base_step % PRESSURE_RING_SIZE) as usize;
        let idx1 = ((base_step + 1) % PRESSURE_RING_SIZE) as usize;

        let left_sample = self.pressure_ring_left[idx]
            + (self.pressure_ring_left[idx1] - self.pressure_ring_left[idx]) * fraction;
        let right_sample = self.pressure_ring_right[idx]
            + (self.pressure_ring_right[idx1] - self.pressure_ring_right[idx]) * fraction;

        self.output_samples_rendered += 1;
        (left_sample, right_sample)
    }

    fn start_voice(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
        velocity: f32,
    ) {
        let note = key.number().unwrap_or(0);
        let minimum_strike_velocity = self.params.minimum_strike_velocity.value() as f64;
        let maximum_strike_velocity = self.params.maximum_strike_velocity.value() as f64;
        let hammer_velocity = (minimum_strike_velocity
            + (maximum_strike_velocity - minimum_strike_velocity) * velocity as f64)
            .clamp(0.05, 12.0);
        let velocity_blend = ((hammer_velocity - minimum_strike_velocity)
            / (maximum_strike_velocity - minimum_strike_velocity).max(1.0e-3))
        .clamp(0.0, 1.0);

        let noise_seed = self
            .next_internal_voice_id
            .wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ ((note as u64) << 32);
        let mut strike_randomness = fastrand::Rng::with_seed(noise_seed);

        let (noise_minimum, noise_maximum) = ordered_range(
            self.params.hammer_noise_min.value() as f64,
            self.params.hammer_noise_max.value() as f64,
        );
        let (unison_minimum, unison_maximum) = ordered_range(
            self.params.unison_width_min.value() as f64,
            self.params.unison_width_max.value() as f64,
        );

        let hammer_noise_level =
            noise_minimum + (noise_maximum - noise_minimum) * strike_randomness.f64();
        let unison_width_level =
            unison_minimum + (unison_maximum - unison_minimum) * strike_randomness.f64();

        let strike_controls = StrikeVoicingControls {
            hammer_hardness: self.params.hammer_hardness.value() as f64,
            hammer_hardness_piano: self.params.hammer_hardness_piano.value() as f64,
            hammer_hardness_mezzo: self.params.hammer_hardness_mezzo.value() as f64,
            hammer_hardness_forte: self.params.hammer_hardness_forte.value() as f64,
            hammer_noise: hammer_noise_level,
            hammer_tone: self.params.hammer_tone.value() as f64,
            soft_pedal: self.params.soft_pedal.value() as f64,
            unison_width: unison_width_level,
            string_length_scale: self.params.string_length.value() as f64,
            strike_point_ratio: self.params.strike_point.value() as f64,
        };

        let design = note_design(note, &strike_controls, velocity_blend);
        let mut engine_voice = PianoVoice::new(&design, self.shared_hammer_geometry);

        for existing_slot in self.voices.iter_mut() {
            let is_same_key = match existing_slot {
                Some(existing) => existing.note == note && existing.channel == channel,
                None => false,
            };
            if is_same_key {
                if let Some(existing) = existing_slot.take() {
                    engine_voice.adopt_string_state_from(&existing.engine);
                    let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                        timing: sample_offset,
                        voice_id: existing.voice_id,
                        channel: existing.channel,
                        key: existing.key,
                    });
                }
            }
        }

        engine_voice.strike(hammer_velocity, noise_seed);

        let new_voice = SynthVoice {
            voice_id,
            internal_voice_id: self.next_internal_voice_id,
            channel,
            key,
            note,
            releasing: false,
            engine: engine_voice,
        };

        self.next_internal_voice_id = self.next_internal_voice_id.wrapping_add(1);
        self.held_keys[(note as usize).min(127)] = true;

        match self.voices.iter().position(|voice| voice.is_none()) {
            Some(free_voice_index) => {
                self.voices[free_voice_index] = Some(new_voice);
            }
            None => {
                let oldest_voice = self.voices.iter_mut().min_by_key(|voice| {
                    voice
                        .as_ref()
                        .map_or(u64::MAX, |candidate| candidate.internal_voice_id)
                });
                if let Some(oldest_slot) = oldest_voice {
                    if let Some(oldest_voice) = oldest_slot.take() {
                        let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                            timing: sample_offset,
                            voice_id: oldest_voice.voice_id,
                            channel: oldest_voice.channel,
                            key: oldest_voice.key,
                        });
                    }
                }
                if let Some(free_voice_index) = self.voices.iter().position(|voice| voice.is_none())
                {
                    self.voices[free_voice_index] = Some(new_voice);
                }
            }
        }
    }

    fn start_release_for_voices(&mut self, voice_id: VoiceID, channel: Channel, key: Key) {
        let note = key.number().unwrap_or(0);
        self.held_keys[(note as usize).min(127)] = false;

        let release_seconds = self.params.damper_release_ms.value() as f64 / 1000.0;

        for voice in self.voices.iter_mut().flatten() {
            let is_target =
                voice_id == voice.voice_id || (channel == voice.channel && key == voice.key);
            if is_target {
                voice.engine.trigger_key_release_noise();
                if !self.sustain_pedal_down {
                    voice.releasing = true;
                    voice
                        .engine
                        .start_release(release_seconds, SIMULATION_STEP_SIZE);
                }
            }
        }
    }

    fn set_sustain_pedal(&mut self, pedal_down: bool) {
        if pedal_down == self.sustain_pedal_down {
            return;
        }
        self.sustain_pedal_down = pedal_down;

        if pedal_down {
            return;
        }

        let release_seconds = self.params.damper_release_ms.value() as f64 / 1000.0;
        for voice in self.voices.iter_mut().flatten() {
            if !self.held_keys[(voice.note as usize).min(127)] {
                voice.releasing = true;
                voice
                    .engine
                    .start_release(release_seconds, SIMULATION_STEP_SIZE);
            }
        }
    }

    fn choke_voices(
        &mut self,
        context: &mut impl ProcessContext<Self>,
        sample_offset: u32,
        voice_id: VoiceID,
        channel: Channel,
        key: Key,
    ) {
        for voice_slot in self.voices.iter_mut() {
            let is_target = match voice_slot {
                Some(voice) => {
                    voice_id == voice.voice_id || (channel == voice.channel && key == voice.key)
                }
                None => false,
            };
            if is_target {
                if let Some(voice) = voice_slot.take() {
                    let _ = context.try_send_event(NoteEvent::VoiceTerminated {
                        timing: sample_offset,
                        voice_id: voice.voice_id,
                        channel: voice.channel,
                        key: voice.key,
                    });
                }
            }
        }
    }
}

fn ordered_range(first: f64, second: f64) -> (f64, f64) {
    if first <= second {
        (first, second)
    } else {
        (second, first)
    }
}

impl ClapPlugin for Prismo {
    const CLAP_ID: &'static str = "com.prismo.prismo";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("A piano-like synth");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
    ];
}

impl Vst3Plugin for Prismo {
    const VST3_CLASS_ID: [u8; 16] = *b"PrismoSynth00001";
    const VST3_SUBCATEGORIES: &'static [Vst3SubCategory] =
        &[Vst3SubCategory::Instrument, Vst3SubCategory::Synth];
}

nice_export_clap!(Prismo);
nice_export_vst3!(Prismo);
