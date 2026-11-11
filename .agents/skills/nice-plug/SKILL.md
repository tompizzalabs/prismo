

## 1. Overview

nice-plug is a Rust framework for building CLAP (and VST3) audio plugins. It provides:

- A `Plugin` trait for audio processing and lifecycle.
- A `ClapPlugin` trait for CLAP metadata and capabilities.
- A derive macro `#[derive(Params)]` for parameter declaration.
- Event-driven MIDI/note processing via `ProcessContext`.
- Polyphonic modulation support (CLAP-specific).
- Optional egui-based editor integration via `nice_plug_egui`.

Export is performed with `nice_export_clap!(PluginType);`.

---

## 2. Minimal Plugin Skeleton

```rust
pub struct MyPlugin {
    params: Arc<MyParams>,
}

impl Default for MyPlugin {
    fn default() -> Self {
        Self { params: Arc::new(MyParams::default()) }
    }
}

impl Plugin for MyPlugin {
    const NAME: &'static str = "My Plugin";
    const VENDOR: &'static str = "Vendor";
    const URL: &'static str = "https://example.com";
    const EMAIL: &'static str = "info@example.com";
    const VERSION: &'static str = env!("CARGO_PKG_VERSION");

    const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    }];

    type Editor = ();             // No GUI
    type SysExMessage = ();       // No SysEx
    type BackgroundTask = ();     // No background task

    fn params(&self) -> Arc<dyn Params> {
        self.params.clone()
    }

    fn process(
        &mut self,
        buffer: &mut Buffer,
        _aux: &mut AuxiliaryBuffers,
        _context: &mut impl ProcessContext<Self>,
    ) -> ProcessStatus {
        // Audio processing
        ProcessStatus::Normal
    }
}

impl ClapPlugin for MyPlugin {
    const CLAP_ID: &'static str = "com.vendor.my-plugin";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Description");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Stereo,
    ];
}

nice_export_clap!(MyPlugin);
```

---

## 3. The `Plugin` Trait

### 3.1 Required Constants

| Constant | Purpose |
|---|---|
| `NAME` | Display name. |
| `VENDOR` | Developer name. |
| `URL` | Product URL. |
| `EMAIL` | Contact email. |
| `VERSION` | Version string. Use `env!("CARGO_PKG_VERSION")`. |
| `AUDIO_IO_LAYOUTS` | Slice of `AudioIOLayout`. First entry is default. |

### 3.2 Associated Types

| Type | Use |
|---|---|
| `Editor` | GUI type. Use `()` when no editor. Use `EguiEditor<App>` for egui editors. |
| `SysExMessage` | SysEx message type. `()` if unused. |
| `BackgroundTask` | Background task type. `()` if unused. |

### 3.3 Optional Constants

| Constant | Default | Notes |
|---|---|---|
| `MIDI_INPUT` | `MidiConfig::None` | Set `MidiConfig::Basic` for notes only. Use fuller config for CC. |
| `SAMPLE_ACCURATE_AUTOMATION` | `false` | Set `true` to receive per-sample automation. Requires manual block splitting. |

### 3.4 Lifecycle Methods

| Method | Invocation | Purpose |
|---|---|---|
| `params()` | Host query | Return `Arc<dyn Params>`. |
| `editor()` | Editor open | Construct editor. Consume stored initial editor via `Option::take()`. |
| `reset()` | Host reset | Restore deterministic state. Clear voices. Reseed PRNG. |
| `activate()` | Before processing | Compute sample-rate-dependent values. Return `bool`. |
| `deactivate()` | After processing | Cleanup. |
| `process()` | Each audio cycle | Core audio and event processing. Return `ProcessStatus`. |

---

## 4. Audio I/O Layouts

```rust
const AUDIO_IO_LAYOUTS: &'static [AudioIOLayout] = &[
    AudioIOLayout {
        main_input_channels: NonZeroU32::new(2),
        main_output_channels: NonZeroU32::new(2),
        ..AudioIOLayout::const_default()
    },
    AudioIOLayout {
        main_input_channels: NonZeroU32::new(1),
        main_output_channels: NonZeroU32::new(1),
        ..AudioIOLayout::const_default()
    },
];
```

- Enumerate all supported channel configurations.
- Instruments may omit input channels.

---

## 5. Parameters

### 5.1 Declaration

```rust
#[derive(Params)]
pub struct GainParams {
    #[id = "gain"]
    pub gain: FloatParam,

    #[id = "foobar"]
    pub some_int: IntParam,
}
```

- `#[id = "..."]` assigns a stable string identifier.
- IDs must remain stable across versions.

### 5.2 FloatParam Construction

```rust
gain: FloatParam::new(
    "Gain",
    util::db_to_gain(0.0),                       // Default plain value
    FloatRange::gain_range(-30.0, 30.0),         // Range
)
.with_smoother(SmoothingStyle::Logarithmic(50.0))
.with_unit(" dB")
.with_value_to_string(formatters::v2s_f32_gain_to_db(2))
.with_string_to_value(formatters::s2v_f32_gain_to_db()),
```

### 5.3 Range Types

| Variant | Use |
|---|---|
| `FloatRange::Linear { min, max }` | Linear mapping. |
| `FloatRange::Skewed { min, max, factor }` | Non-linear mapping via `FloatRange::skew_factor(-1.0)`. |
| `FloatRange::gain_range(min_db, max_db)` | Gain in dB, logarithmic. |

### 5.4 IntParam

```rust
some_int: IntParam::new("Something", 3, IntRange::Linear { min: 0, max: 3 }),
```

### 5.5 Builder Methods

| Method | Effect |
|---|---|
| `.with_smoother(SmoothingStyle)` | Enable smoothing. Styles: `Linear(ms)`, `Logarithmic(ms)`, `Exponential(ms)`, `None`. |
| `.with_unit(" dB")` | Display unit. |
| `.with_value_to_string(fn)` | Plain value to display string. |
| `.with_string_to_value(fn)` | Parse string to plain value. |
| `.with_step_size(f32)` | Step increment. |
| `.with_poly_modulation_id(u32)` | Enable CLAP polyphonic modulation for this parameter. |

### 5.6 Smoothing Access

| Call | Behavior |
|---|---|
| `param.value()` | Current plain unsmoothed value. |
| `param.smoothed.next()` | Next smoothed sample. |
| `param.smoothed.next_block(&mut buf, len)` | Fill buffer with smoothed values. |
| `param.preview_modulated(offset)` | Plain value at normalized modulation offset. |
| `param.preview_plain(normalized)` | Plain value at normalized position. |
| `param.smoothed.clone()` | Clone smoother state into a voice. |

---

## 6. Audio Processing

### 6.1 Buffer API

```rust
fn process(
    &mut self,
    buffer: &mut Buffer,
    _aux: &mut AuxiliaryBuffers,
    context: &mut impl ProcessContext<Self>,
) -> ProcessStatus {
    for channel_samples in buffer.iter_samples() {
        let gain = self.params.gain.smoothed.next();
        for sample in channel_samples {
            *sample *= gain;
        }
    }
    ProcessStatus::Normal
}
```

- `buffer.samples()` returns sample count.
- `buffer.as_slice()` returns `&mut [&mut [f32]]` for raw channel access.
- Return `ProcessStatus::Normal`, `ProcessStatus::Tail`, or `ProcessStatus::Sleep`.

### 6.2 Manual Block Splitting

Required for sample-accurate events in instruments. Pattern:

```rust
const MAX_BLOCK_SIZE: usize = 64;

let num_samples = buffer.samples();
let output = buffer.as_slice();
let mut next_event = context.next_event();
let mut block_start: usize = 0;
let mut block_end: usize = MAX_BLOCK_SIZE.min(num_samples);

while block_start < num_samples {
    // 1. Handle events at block_start.
    // 2. Truncate block_end if an event occurs before it.
    // 3. Render audio for [block_start..block_end].
    block_start = block_end;
    block_end = (block_start + MAX_BLOCK_SIZE).min(num_samples);
}
```

Event loop within each block:

```rust
'events: loop {
    match next_event {
        Some(event) if (event.timing() as usize) <= block_start => {
            match event {
                NoteEvent::NoteOn { .. } => { /* ... */ }
                NoteEvent::NoteOff { .. } => { /* ... */ }
                NoteEvent::Choke { .. } => { /* ... */ }
                NoteEvent::PolyModulation { .. } => { /* ... */ }
                NoteEvent::MonoAutomation { .. } => { /* ... */ }
                _ => (),
            }
            next_event = context.next_event();
        }
        Some(event) if (event.timing() as usize) < block_end => {
            block_end = event.timing() as usize;
            break 'events;
        }
        _ => break 'events,
    }
}
```

---

## 7. Note Events

### 7.1 Input Events

| Variant | Fields | Action |
|---|---|---|
| `NoteOn` | `timing, voice_id, channel, key, velocity` | Start voice. |
| `NoteOff` | same | Begin release. |
| `Choke` | same | Immediate termination. |
| `PolyModulation` | `timing, voice_id, poly_modulation_id, normalized_offset` | Apply per-voice modulation. |
| `MonoAutomation` | `timing, poly_modulation_id, normalized_value` | Update global automation for modulated voices. |

### 7.2 Output Events

Send via `context.try_send_event(event)`. Result must be handled.

```rust
context.try_send_event(NoteEvent::VoiceTerminated {
    timing: block_end as u32,
    voice_id: VoiceID::ID(voice.voice_id),
    channel: voice.channel,
    key: voice.key,
}).unwrap();
```

`VoiceTerminated` is mandatory whenever a polyphonically modulated voice ends. The host relies on it to free modulation resources.

### 7.3 Voice ID Resolution

```rust
let resolved: i32 = voice_id.id_or_fallback(key, channel);
```

- Use host-provided ID when present.
- Fallback IDs disable polyphonic modulation but preserve note handling.
- `voice_id.is_wildcard()` indicates matching by channel/key instead of ID.

---

## 8. Polyphonic Modulation

### 8.1 Parameter Registration

```rust
const GAIN_POLY_MOD_ID: u32 = 0;

gain: FloatParam::new(/* ... */)
    .with_poly_modulation_id(GAIN_POLY_MOD_ID)
    .with_smoother(SmoothingStyle::Logarithmic(5.0)),
```

### 8.2 CLAP Configuration

```rust
const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> =
    Some(PolyModulationConfig {
        max_voice_capacity: NUM_VOICES,
        supports_overlapping_voices: true,
    });
```

- Set `None` when polyphonic modulation is not supported.
- If voice capacity changes at runtime, call `context.set_current_voice_capacity()` in `activate()` and in `process()` upon change.

### 8.3 Per-Voice Modulation State

Store an optional smoother per voice:

```rust
voice_gain: Option<(f32, Smoother<f32>)>, // (normalized_offset, smoother)
```

Handling `PolyModulation`:

```rust
NoteEvent::PolyModulation {
    voice_id, poly_modulation_id, normalized_offset, ..
} => {
    if let Some(idx) = self.get_voice_idx(voice_id) {
        let voice = self.voices[idx].as_mut().unwrap();
        if poly_modulation_id == GAIN_POLY_MOD_ID {
            let target = self.params.gain.preview_modulated(normalized_offset);
            let (_, smoother) = voice.voice_gain.get_or_insert_with(|| {
                (normalized_offset, self.params.gain.smoothed.clone())
            });
            if voice.internal_voice_id >= this_sample_internal_voice_id_start {
                smoother.reset(target);      // Same sample as NoteOn
            } else {
                smoother.set_target(sample_rate, target);
            }
        }
    }
}
```

Handling `MonoAutomation`:

```rust
NoteEvent::MonoAutomation {
    poly_modulation_id, normalized_value, ..
} => {
    for voice in self.voices.iter_mut().filter_map(|v| v.as_mut()) {
        if poly_modulation_id == GAIN_POLY_MOD_ID {
            if let Some((offset, smoother)) = voice.voice_gain.as_mut() {
                let target = self.params.gain
                    .preview_plain(normalized_value + *offset);
                smoother.set_target(sample_rate, target);
            }
        }
    }
}
```

Rules:

1. Modulation is always an offset to the automated value.
2. Voices without modulation use the global smoothed parameter.
3. Voices with modulation render from their private smoother.

```rust
let gain = match &voice.voice_gain {
    Some((_, smoother)) => {
        smoother.next_block(&mut voice_gain_buf, block_len);
        &voice_gain_buf
    }
    None => &global_gain_buf,
};
```

---

## 9. Voice Management

### 9.1 Voice Struct

```rust
#[derive(Debug, Clone)]
struct Voice {
    voice_id: i32,
    channel: Channel,
    key: Key,
    internal_voice_id: u64,
    velocity_sqrt: f32,
    phase: f32,
    phase_delta: f32,
    releasing: bool,
    amp_envelope: Smoother<f32>,
    voice_gain: Option<(f32, Smoother<f32>)>,
}
```

### 9.2 Allocation

```rust
voices: [Option<Voice>; NUM_VOICES as usize],
```

Initialize with `[0; N].map(|_| None)`.

### 9.3 Voice Stealing

When no free slot exists:

1. Find voice with minimum `internal_voice_id`.
2. Emit `VoiceTerminated` for the stolen voice.
3. Overwrite slot with the new voice.

Increment `next_internal_voice_id` with `wrapping_add(1)` per allocation.

### 9.4 Envelope via Smoother

```rust
let env = Smoother::new(SmoothingStyle::Exponential(attack_ms));
env.reset(0.0);
env.set_target(sample_rate, 1.0);
```

Release:

```rust
voice.amp_envelope.style = SmoothingStyle::Exponential(release_ms);
voice.amp_envelope.set_target(sample_rate, 0.0);
```

Termination condition:

```rust
if v.releasing && v.amp_envelope.previous_value() == 0.0 {
    // Send VoiceTerminated, set slot to None
}
```

---

## 10. Activation and Sample-Rate-Dependent State

```rust
fn activate(
    &mut self,
    _layout: &AudioIOLayout,
    buffer_config: &BufferConfig,
    _context: &mut impl ActivateContext<Self>,
) -> bool {
    self.peak_meter_decay_weight = 0.25f64
        .powf((buffer_config.sample_rate as f64 * DECAY_MS / 1000.0).recip())
        as f32;
    true
}
```

Return `true` on success.

---

## 11. GUI with egui (`nice_plug_egui`)

### 11.1 Types

| Type | Role |
|---|---|
| `EguiEditor<App>` | Editor handle returned by `editor()`. |
| `EguiEditorState` | Persistent size/zoom state. Construct with `from_size(LogicalSize, zoom)`. |
| `NiceEguiApp` | Trait implemented by the app struct. |
| `RepaintNotifier` | Realtime-safe repaint request handle. |
| `ResizableWindow` | Resizable container widget. |
| `widgets::ParamSlider` | Parameter-aware slider. |

### 11.2 App Struct

```rust
pub struct GainEditor {
    open_state: Option<OpenEditorState>,
    params: Arc<GainParams>,
    peak_meter: Arc<AtomicF32>,
    msg_channel: GuiMsgChannel,
    triple_buffer_state: triple_buffer::Input<TripleBufferState>,
    zoom_factor: f32,
    track_info: Option<TrackInfo>,
}
```

State persists across editor open/close cycles.

### 11.3 NiceEguiApp Implementation

```rust
impl NiceEguiApp for GainEditor {
    fn build(
        &mut self,
        egui_ctx: egui::Context,
        nice_gui_ctx: GuiContext,
        _frame: &mut nice_plug_egui::Frame,
    ) -> Result<(), nice_plug_egui::baseview::HandlerError> {
        self.open_state = Some(OpenEditorState::new(egui_ctx, nice_gui_ctx));
        Ok(())
    }

    fn editor_closed(&mut self) {
        self.open_state = None;   // Drop all egui::Context copies
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut nice_plug_egui::Frame) {
        // Draw widgets
    }

    fn track_info_changed(&mut self, info: TrackInfo) {
        self.track_info = Some(info);
    }
}
```

Rules:

1. Store `egui::Context` only inside `open_state`.
2. Clear it in `editor_closed()`. Editors open and close repeatedly.
3. Use `state.nice_gui_ctx.param_setter()` for parameter writes.

### 11.4 Editor Construction

```rust
type Editor = EguiEditor<GainEditor>;

fn editor(&mut self, _exec: AsyncExecutor<Self>) -> Option<Self::Editor> {
    create_egui_editor(
        self.editor_state.clone(),
        self.repaint_notifier.clone(),
        EguiNiceSettings::new().with_resize_hint(RESIZE_HINT),
        self.initial_editor.take().unwrap(),
    )
}
```

Resize hint:

```rust
const MIN_WINDOW_SIZE: LogicalSize<f32> = LogicalSize::new(300.0, 300.0);
const RESIZE_HINT: ResizeHint =
    ResizeHint::resizable().with_min_logical_size(MIN_WINDOW_SIZE);
```

### 11.5 Parameter Widgets

Parameter-aware slider:

```rust
ui.add(widgets::ParamSlider::for_param(&self.params.gain, &setter));
```

Manual slider with gesture tracking:

```rust
let prev_value = util::gain_to_db(self.params.gain.value());
let mut new_value = prev_value;
let ptr_down = ui.add(
    egui::widgets::Slider::new(&mut new_value, -30.0..=30.0).suffix(" dB"),
).is_pointer_button_down_on();

if !state.is_dragging_slider && (ptr_down || new_value != prev_value) {
    state.is_dragging_slider = true;
    setter.begin_set_parameter(&self.params.gain);
}
if new_value != prev_value {
    setter.set_parameter(&self.params.gain, util::db_to_gain(new_value));
}
if state.is_dragging_slider && !ptr_down {
    state.is_dragging_slider = false;
    setter.end_set_parameter(&self.params.gain);
}
```

Gesture protocol: `begin_set_parameter` → N × `set_parameter` → `end_set_parameter`.

### 11.6 Zoom

```rust
egui::ComboBox::from_label("zoom factor")
    .selected_text(format!("{}%", (self.zoom_factor * 100.0).round() as u32))
    .show_ui(ui, |ui| {
        ui.selectable_value(&mut self.zoom_factor, 1.0, "100%");
        // ...
    });
if self.zoom_factor != before {
    ui.set_zoom_factor(self.zoom_factor);
}
```

Persist zoom in a config file. Window size is managed by the host.

### 11.7 Track Information

```rust
let info = self.track_info.clone().unwrap_or_default();
let name = info.name();
if let Some(color) = info.color() {
    let (r, g, b, a) = color.rgba();
    // Paint swatch with egui::Color32::from_rgba_unmultiplied(r, g, b, a)
}
```

---

## 12. GUI ↔ Audio Thread Communication

### 12.1 Message Channels (rtrb)

```rust
const GUI_TO_AUDIO_CAPACITY: usize = 512;
const AUDIO_TO_GUI_CAPACITY: usize = 128;

let (to_audio_tx, from_gui_rx) = rtrb::RingBuffer::new(GUI_TO_AUDIO_CAPACITY);
let (to_gui_tx, from_audio_rx) = rtrb::RingBuffer::new(AUDIO_TO_GUI_CAPACITY);
```

Define message enums:

```rust
pub enum GuiToAudioMsg {
    MessageA,
    MessageWithHeapData(Vec<f32>),
}

pub enum AudioToGuiMsg {
    MessageA,
    DropOldHeapData(Vec<f32>),
}
```

GUI send:

```rust
if let Err(e) = self.msg_channel.to_audio_tx.push(GuiToAudioMsg::MessageA) {
    nice_error!("Send failed: {}", e);
}
```

Audio receive:

```rust
while let Ok(msg) = self.msg_channel.from_gui_rx.pop() {
    match msg { /* handle */ }
}
```

### 12.2 Heap Data Rules

1. Never drop heap allocations on the audio thread.
2. Swap new data in. Send the old data back to the GUI thread for deallocation.

```rust
GuiToAudioMsg::MessageWithHeapData(mut new_data) => {
    std::mem::swap(&mut self.heap_data_example, &mut new_data);
    self.msg_channel.to_gui_tx
        .push(AudioToGuiMsg::DropOldHeapData(new_data))
        .ok();
}
```

### 12.3 Triple Buffer

Alternative to channels. 3× memory cost. Full overwrite required.

```rust
let (input, output) = triple_buffer::triple_buffer(&TripleBufferState::default());

// Writer side (GUI):
self.triple_buffer_state.write(TripleBufferState {
    value_a: false,
    value_b: self.next_value,
    some_data: Vec::new(),
});

// Reader side (audio):
let state = self.triple_buffer_state.read();
```

Note: `input_buffer_mut()` must not be used for this pattern. Always write a complete new state.

### 12.4 Shared Atomics

```rust
let peak_meter: Arc<AtomicF32> = Arc::new(AtomicF32::new(util::MINUS_INFINITY_DB));

// Audio thread write:
self.peak_meter.store(new_value, Ordering::Relaxed);
self.repaint_notifier.request_repaint();

// GUI thread read:
let value = self.peak_meter.load(Ordering::Relaxed);
ui.request_repaint();  // Continue smoothing next frame
```

### 12.5 Repaint Notification

`RepaintNotifier` is realtime-safe. Use it from the audio thread when GUI-visible data changes. Gate expensive GUI computations behind `self.editor_state.is_open()`.

---

## 13. Utility Functions

| Function | Purpose |
|---|---|
| `util::db_to_gain(db)` | dB to linear gain. |
| `util::gain_to_db(gain)` | Linear gain to dB. |
| `util::midi_key_to_freq(key)` | MIDI note to Hz. |
| `util::MINUS_INFINITY_DB` | Negative infinity dB constant. |
| `util::MINUS_INFINITY_GAIN` | Zero gain constant. |
| `formatters::v2s_f32_gain_to_db(decimals)` | Value-to-string for gain params. |
| `formatters::s2v_f32_gain_to_db()` | String-to-value for gain params. |

---

## 14. CLAP Trait Reference

```rust
impl ClapPlugin for Gain {
    const CLAP_ID: &'static str = "com.vendor.plugin-id";
    const CLAP_DESCRIPTION: Option<&'static str> = Some("Description");
    const CLAP_MANUAL_URL: Option<&'static str> = Some(Self::URL);
    const CLAP_SUPPORT_URL: Option<&'static str> = None;
    const CLAP_FEATURES: &'static [ClapFeature] = &[
        ClapFeature::AudioEffect,
        ClapFeature::Instrument,
        ClapFeature::Synthesizer,
        ClapFeature::Stereo,
        ClapFeature::Mono,
        ClapFeature::Utility,
    ];
    const CLAP_POLY_MODULATION_CONFIG: Option<PolyModulationConfig> = None;
}
```

- `CLAP_ID` must be globally unique.
- `CLAP_POLY_MODULATION_CONFIG` is optional. Include only when polyphonic modulation is implemented.

---

## 15. Export Macros

```rust
nice_export_clap!(MyPlugin);
nice_export_vst3!(MyPlugin);   // Optional, requires Vst3Plugin impl
```

---

## 16. Determinism and Reset

```rust
fn reset(&mut self) {
    self.prng = Pcg32::new(420, 1337);  // Fixed seed
    self.voices.fill(None);
    self.next_internal_voice_id = 0;
}
```

Fixed-seed PRNG guarantees identical output across offline renders.

---

## 17. Best Practices

1. Audio thread: no allocation, no deallocation, no locks, no I/O.
2. Route all heap cleanup to the GUI thread.
3. Handle all `Result` values from channel pushes and `try_send_event`.
4. Send `VoiceTerminated` for every terminated modulated voice, including stolen voices.
5. Split blocks at event boundaries when `SAMPLE_ACCURATE_AUTOMATION` is enabled.
6. Guard GUI-only computations with `editor_state.is_open()`.
7. Clone smoothers into voices for per-voice modulation instead of recomputing coefficients.
8. Use `Relaxed` ordering for atomic meter data.
9. Drop all `egui::Context` copies in `editor_closed()`.
10. Keep parameter IDs stable.
11. Precompute sample-rate-dependent weights in `activate()`.
12. Size ring buffers for worst-case message volume.

---

## 18. Common Failure Modes

| Failure | Cause | Correction |
|---|---|---|
| Modulation applied to wrong voice | Matching by channel/key instead of voice ID. | Match on `voice_id` first. |
| Modulation ignored on new notes | Smoother initialized from global value. | `smoother.reset(target)` when `internal_voice_id >= block_start_id`. |
| Host modulation leak | Missing `VoiceTerminated`. | Emit on release end, choke, and voice steal. |
| Audio thread stall | Heap deallocation in `process()`. | Return allocations to GUI via channel. |
| Editor state corruption | Context retained after close. | Null `open_state` in `editor_closed()`. |
| Stale meter display | Missing repaint request. | Call `repaint_notifier.request_repaint()` or `ui.request_repaint()`. |
| Clicks on automation | No block splitting. | Cut blocks at event timing. |
```
