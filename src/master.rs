//! Output master stage: headroom trim, DC removal, and a stereo-linked soft
//! peak limiter.
//!
//! The physical simulation produces raw pressure values whose peak magnitude
//! depends on the played dynamics and polyphony. This stage guarantees the final
//! output never exceeds the limiter ceiling (so the plugin can never digitally
//! clip), while keeping the default level conservative through a fixed headroom
//! trim. The limiter only engages on extreme material (full-polyphony
//! fortissimos) and uses a fast attack / slow release so its action is
//! transparent in normal playing.

/// Fixed loudness headroom applied at unity gain, so normal playing sits well
/// below full scale and the limiter only touches extreme peaks.
pub const OUTPUT_HEADROOM_DB: f64 = -3.0;

/// Hard ceiling for the limited output, in dBFS.
pub const LIMITER_CEILING_DB: f64 = -0.5;

/// Attack time constant of the limiter gain reduction (seconds).
pub const LIMITER_ATTACK_SECONDS: f64 = 0.5e-3;

/// Release time constant of the limiter gain recovery (seconds).
pub const LIMITER_RELEASE_SECONDS: f64 = 0.18;

/// Cutoff frequency of the DC-blocking high-pass filter (Hz).
pub const DC_BLOCK_CUTOFF_HZ: f64 = 3.0;

#[derive(Clone)]
pub struct OutputMaster {
    headroom: f64,
    ceiling: f64,
    attack_weight: f64,
    release_weight: f64,
    dc_retain: f64,
    limiter_gain: f64,
    dc_state_left: f64,
    dc_state_right: f64,
    previous_input_left: f64,
    previous_input_right: f64,
}

impl Default for OutputMaster {
    fn default() -> Self {
        Self::new(48_000.0)
    }
}

impl OutputMaster {
    pub fn new(sample_rate: f64) -> Self {
        let sample_rate = sample_rate.max(1.0);

        let prepare_weight = |time_constant: f64| -> f64 {
            if time_constant <= 0.0 {
                1.0
            } else {
                1.0 - (-1.0 / (time_constant * sample_rate)).exp()
            }
        };

        OutputMaster {
            headroom: 10.0_f64.powf(OUTPUT_HEADROOM_DB / 20.0),
            ceiling: 10.0_f64.powf(LIMITER_CEILING_DB / 20.0),
            attack_weight: prepare_weight(LIMITER_ATTACK_SECONDS),
            release_weight: prepare_weight(LIMITER_RELEASE_SECONDS),
            dc_retain: (-2.0 * std::f64::consts::PI * DC_BLOCK_CUTOFF_HZ / sample_rate)
                .exp()
                .clamp(0.99, 1.0 - 1.0e-6),
            limiter_gain: 1.0,
            dc_state_left: 0.0,
            dc_state_right: 0.0,
            previous_input_left: 0.0,
            previous_input_right: 0.0,
        }
    }

    /// Process one stereo sample (already multiplied by the user gain) and
    /// return the mastered pair.
    #[inline]
    pub fn process(&mut self, left_input: f64, right_input: f64) -> (f64, f64) {
        // Headroom: bring the nominal playing range down into a safe zone.
        let left_scaled = left_input * self.headroom;
        let right_scaled = right_input * self.headroom;

        // DC removal (one-pole high-pass per channel).
        let left_hp = left_scaled - self.previous_input_left + self.dc_retain * self.dc_state_left;
        self.previous_input_left = left_scaled;
        self.dc_state_left = left_hp;
        let right_hp = right_scaled - self.previous_input_right + self.dc_retain * self.dc_state_right;
        self.previous_input_right = right_scaled;
        self.dc_state_right = right_hp;

        // Stereo-linked peak detection.
        let peak = left_hp.abs().max(right_hp.abs());
        let desired_gain = {
            if peak > self.ceiling {
                self.ceiling / peak
            } else {
                1.0
            }
        };

        // Smooth the gain reduction: fast when clamping down, slow on recovery.
        let weight = if desired_gain < self.limiter_gain {
            self.attack_weight
        } else {
            self.release_weight
        };
        self.limiter_gain += (desired_gain - self.limiter_gain) * weight;

        let left_out = left_hp * self.limiter_gain;
        let right_out = right_hp * self.limiter_gain;

        // Absolute safety clamp: no sample can exceed the ceiling.
        let mut left_out = left_out.clamp(-self.ceiling, self.ceiling);
        let mut right_out = right_out.clamp(-self.ceiling, self.ceiling);

        // The Clap/VST3 backends expect finite samples; guard any residual NaN.
        if !left_out.is_finite() {
            left_out = 0.0;
        }
        if !right_out.is_finite() {
            right_out = 0.0;
        }

        (left_out, right_out)
    }

    pub fn reset(&mut self) {
        self.limiter_gain = 1.0;
        self.dc_state_left = 0.0;
        self.dc_state_right = 0.0;
        self.previous_input_left = 0.0;
        self.previous_input_right = 0.0;
    }
}
