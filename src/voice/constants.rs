//! Tuning constants for the piano voice model.

/// Number of coupling sweeps used while the hammer is in contact with the strings.
pub const SUB_STEPS_DURING_STRIKE: usize = 32;

/// How far the hammer is allowed to rest below its contact angle.
pub(super) const HAMMER_REST_ANGLE_DROP: f64 = 0.25;

/// Angular margin below the contact angle before strike phase is considered active.
pub(super) const STRIKE_PHASE_ANGLE_MARGIN: f64 = 0.05;

/// Small lateral offsets between unison strings relative to the hammer center.
pub(super) const UNISON_LATERAL_MISALIGNMENTS: [f64; 3] = [0.28e-3, -0.22e-3, 0.34e-3];

/// Maximum number of unison strings supported by the voice.
pub(super) const MAXIMUM_UNISON_STRINGS: usize = 3;

/// Duration of the hammer impact noise burst in seconds.
pub(super) const HAMMER_NOISE_WINDOW_SECONDS: f64 = 12.0e-3;

/// Time constant for the hammer noise decay envelope in seconds.
pub(super) const HAMMER_NOISE_DECAY_SECONDS: f64 = 2.5e-3;

/// Scales the part of the hammer thump that reaches the bridge.
pub(super) const HAMMER_THUMP_BRIDGE_SCALE: f64 = 0.12;

/// Duration of the key release (damper landing) noise burst in seconds.
pub(super) const KEY_RELEASE_NOISE_WINDOW_SECONDS: f64 = 0.030;

/// Time constant for the key release noise decay envelope in seconds.
pub(super) const KEY_RELEASE_NOISE_DECAY_SECONDS: f64 = 0.009;

/// Peak amplitude of the key release noise.
pub(super) const KEY_RELEASE_NOISE_AMPLITUDE: f64 = 0.05;

/// Cutoff frequency of the low-pass filter applied to key release noise.
pub(super) const KEY_RELEASE_NOISE_CUTOFF_HZ: f64 = 850.0;

/// Frequency ratios above the fundamental used by the duplex resonators.
pub(super) const DUPLEX_FREQUENCY_RATIOS: [f64; 6] = [8.53, 10.47, 12.61, 15.13, 18.09, 21.47];

/// Modal mass of each duplex resonator.
pub(super) const DUPLEX_MODAL_MASS: f64 = 1.0e-3;

/// Damping of each duplex resonator.
pub(super) const DUPLEX_MODAL_DAMPING: f64 = 5.0e-4;

/// Scales the bridge displacement drive fed into the duplex resonators.
pub(super) const DUPLEX_DRIVE_SCALE: f64 = 1.0e-3;

/// Scales the duplex resonators' reaction force back onto the bridge.
pub(super) const DUPLEX_OUTPUT_SCALE: f64 = 0.05;

/// Natural logarithm of the damper amplitude decay factor over the release window.
pub(super) const DAMPER_AMPLITUDE_DECAY_LN: f64 = -6.907755278982137;
