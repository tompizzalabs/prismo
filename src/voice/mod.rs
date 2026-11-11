//! Voice management: polyphonic engine voices and their synthesis.
//!
//! Entry point for the `voice` module, re-exporting the public types and
//! constants used by the rest of the plugin.

mod advance;
mod constants;
mod state;
mod strike;

pub use constants::SUB_STEPS_DURING_STRIKE;

use constants::MAXIMUM_UNISON_STRINGS;
use crate::engine::math::vector::Vector3;
use crate::engine::osc::hammer::Hammer;
use crate::engine::osc::modal::ModalOscillator;
use crate::engine::osc::strings::KeysString;
use nice_plug::midi::{Channel, Key, VoiceID};

/// A single note's engine state: unison strings, hammer, dampers, and duplex resonators.
pub struct PianoVoice {
    pub strings: Vec<KeysString>,
    pub hammer: Hammer,
    strike_velocity: f64,
    release_noise_seconds_remaining: f64,
    release_noise_amplitude: f64,
    release_noise_filter_state: f64,
    release_noise_random_state: u64,
    previous_felt_compressions: [Vector3; MAXIMUM_UNISON_STRINGS],
    felt_contact_references: [Vector3; MAXIMUM_UNISON_STRINGS],
    grazing_contact_angle: f64,
    strike_activation_angle: f64,
    reference_lateral_offset: f64,
    hammer_force_scales: [f64; MAXIMUM_UNISON_STRINGS],
    damper_decay_per_step: f64,
    hammer_noise_seconds_remaining: f64,
    hammer_noise_amplitude: f64,
    hammer_noise_cutoff_hz: f64,
    hammer_noise_filter_state: f64,
    hammer_noise_random_state: u64,
    note_age_seconds: f64,
    blooming_envelope: f64,
    bridge_coupling_factor: f64,
    duplex_output_scale: f64,
    duplex_resonators: Vec<ModalOscillator>,
    duplex_reaction_coefficients: Vec<f64>,
    snapshot_buffer: Vec<f64>,
}

/// A voice that is being tracked by a MIDI note with a matching synth engine.
pub struct SynthVoice {
    pub voice_id: VoiceID,
    pub internal_voice_id: u64,
    pub channel: Channel,
    pub key: Key,
    pub note: u8,
    pub releasing: bool,
    pub engine: PianoVoice,
}
