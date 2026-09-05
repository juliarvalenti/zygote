use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Disableable, Root, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use zygote_core::{
    DEFAULT_PORT, GateEvent, GateLog, Graph, GraphStructure, KeyAction, LfoShape, Message,
    ModContext, ModSource, NodeId, NodeLibrary, PROTOCOL_VERSION, ParamDescriptor, ParamPath,
    ParamSender, ParamType, ParamValue, SourceKind, Timeline, Transition,
    WindowBounds as OutputBounds, apply_offset,
};

/// Keys owned by the transport; key learn refuses them.
const RESERVED_KEYS: &[&str] = &[
    "space", "escape", "home", "[", "]", "enter", "l", "e", "t", "shift-t",
];

/// UI refresh / send rate.
const TICK: Duration = Duration::from_millis(33);
/// How long to wait for the renderer's `Describe` before falling back to the
/// built-in first-pass graph for the parameter list.
const DESCRIBE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Re-send `Hello`/`Ping` this often.
const HELLO_INTERVAL: Duration = Duration::from_secs(2);
const AXIS_HEIGHT: f32 = 104.0;
const RAIL_WIDTH: f32 = 200.0;
const NODE_W: f32 = 168.0;
const NODE_H: f32 = 46.0;
const COL_GAP: f32 = 48.0;
const ROW_GAP: f32 = 14.0;
const GRAPH_PAD: f32 = 16.0;
const GRAPH_VIEW_H: f32 = 150.0;

// Transport keys. Handled in the capture phase at the window root so no
// focused control can swallow them.
actions!(
    zygote_timeline,
    [
        TogglePlay, Stop, GoToStart, PrevCue, NextCue, AddCue, ToggleLoop, ToggleLive, TileOutput,
        PopOut
    ]
);

struct Options {
    file: PathBuf,
    target: String,
}

fn parse_args() -> Options {
    let mut options = Options {
        file: PathBuf::from("zygote-timeline.json"),
        target: format!("127.0.0.1:{DEFAULT_PORT}"),
    };
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--target" => {
                options.target = args
                    .next()
                    .unwrap_or_else(|| usage("--target needs host:port"));
            }
            "-h" | "--help" => usage(""),
            path if !path.starts_with('-') => options.file = PathBuf::from(path),
            other => usage(&format!("unknown argument {other}")),
        }
    }
    options
}

fn usage(error: &str) -> ! {
    if !error.is_empty() {
        eprintln!("error: {error}\n");
    }
    eprintln!("usage: zygote-timeline [timeline.json] [--target host:port]");
    std::process::exit(if error.is_empty() { 0 } else { 2 });
}

pub fn run() {
    let options = parse_args();
    gpui_kit::application().run(move |cx| {
        gpui_kit::init(cx);
        cx.bind_keys([
            KeyBinding::new("space", TogglePlay, None),
            KeyBinding::new("escape", Stop, None),
            KeyBinding::new("home", GoToStart, None),
            KeyBinding::new("[", PrevCue, None),
            KeyBinding::new("]", NextCue, None),
            KeyBinding::new("enter", AddCue, None),
            KeyBinding::new("l", ToggleLoop, None),
            KeyBinding::new("e", ToggleLive, None),
            KeyBinding::new("t", TileOutput, None),
            KeyBinding::new("shift-t", PopOut, None),
        ]);

        // Take the left part of the primary display so the output window can
        // be tiled to the right of us.
        let bounds = match cx.primary_display().map(|d| d.bounds()) {
            Some(display) => {
                let width = (display.size.width * 0.46).max(px(900.));
                Bounds {
                    origin: display.origin + point(px(12.), px(40.)),
                    size: size(width, display.size.height - px(80.)),
                }
            }
            None => Bounds::centered(None, size(px(1040.), px(760.)), cx),
        };
        cx.open_window(
            WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                titlebar: Some(TitlebarOptions {
                    title: Some("Zygote Timeline".into()),
                    ..Default::default()
                }),
                focus: true,
                ..Default::default()
            },
            |window, cx| {
                let view = cx.new(|cx| TimelineApp::new(options, window, cx));
                cx.new(|cx| Root::new(view, window, cx))
            },
        )
        .expect("failed to open window");
        cx.on_window_closed(|cx, _| cx.quit()).detach();
        cx.activate(true);
    });
}

/// One control bound to a parameter. Floats and ints get a slider, vec2 and
/// color get one slider per component, bools a switch, choices a button row.
struct ParamControl {
    desc: ParamDescriptor,
    sliders: Vec<Entity<SliderState>>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    Playhead,
    Cue(u32),
}

/// What moving a control means right now.
/// What the next key press will be bound to.
#[derive(Clone, Debug, PartialEq, Eq)]
enum LearnTarget {
    Trigger(String),
    Cue(u32),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Stopped and parked on a cue: controls write into that cue.
    Edit(u32),
    /// Playing, or not on a cue: controls are live overrides on top of the cues.
    Live,
}

pub struct TimelineApp {
    file: PathBuf,
    focus_handle: FocusHandle,
    graph: Option<GraphStructure>,
    show_graph: bool,
    graph_pan: Point<f32>,
    graph_drag: Option<(Point<Pixels>, Point<f32>)>,
    graph_viewport: Rc<Cell<Bounds<Pixels>>>,
    graph_fit_pending: bool,
    /// Rail selection: show only this node's parameters.
    selected_node: Option<NodeId>,
    timeline: Timeline,
    playhead: f32,
    playing: bool,
    /// Force live mode even when parked on a cue.
    force_live: bool,
    selected: Option<u32>,
    params: Vec<ParamControl>,
    overrides: BTreeMap<ParamPath, ParamValue>,
    sent: BTreeMap<ParamPath, ParamValue>,
    sender: Option<ParamSender>,
    described: bool,
    describe_buffer: Vec<ParamDescriptor>,
    started: Instant,
    last_hello: Instant,
    last_heard: Option<Instant>,
    quiet: bool,
    protocol_mismatch: Option<u32>,
    last_tick: Instant,
    drag: Drag,
    axis_bounds: Rc<Cell<Bounds<Pixels>>>,
    tiled: bool,
    /// The timeline is the built-in demo, not a file; it may be replaced by a
    /// fresh one once the renderer describes a graph it does not fit.
    timeline_is_default: bool,
    status: String,
    /// Gate events, mirrored to the renderer; envelopes are evaluated from it.
    gates: GateLog,
    /// The modulation setup changed and must be re-sent.
    mod_dirty: bool,
    /// Parameter whose assignment editor is open.
    mod_editor: Option<ParamPath>,
    depth_slider: Option<(ParamPath, Entity<SliderState>, Subscription)>,
    /// Per-source parameter sliders (rate/phase or A/D/S/R).
    source_sliders: BTreeMap<String, (Vec<Entity<SliderState>>, Vec<Subscription>)>,
    learning: Option<LearnTarget>,
    held_keys: BTreeSet<String>,
}

impl TimelineApp {
    fn new(options: Options, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (timeline, status, timeline_is_default) = match std::fs::read_to_string(&options.file) {
            Ok(json) => match Timeline::from_json(&json) {
                Ok(t) => (t, format!("loaded {}", options.file.display()), false),
                Err(e) => (
                    default_timeline(),
                    format!("could not parse {}: {e}", options.file.display()),
                    true,
                ),
            },
            Err(_) => (default_timeline(), "new timeline".to_owned(), true),
        };

        let sender = match ParamSender::connect(options.target.as_str()) {
            Ok(s) => {
                let _ = s.send(&Message::hello("zygote-timeline"));
                Some(s)
            }
            Err(e) => {
                eprintln!("cannot open udp socket towards {}: {e}", options.target);
                None
            }
        };

        let focus_handle = cx.focus_handle();
        window.focus(&focus_handle, cx);

        let mut this = Self {
            file: options.file,
            focus_handle,
            graph: None,
            show_graph: false,
            graph_pan: Point::default(),
            graph_drag: None,
            graph_viewport: Rc::new(Cell::new(Bounds::default())),
            graph_fit_pending: true,
            selected_node: None,
            timeline,
            playhead: 0.0,
            playing: false,
            force_live: false,
            selected: None,
            params: Vec::new(),
            overrides: BTreeMap::new(),
            sent: BTreeMap::new(),
            sender,
            described: false,
            describe_buffer: Vec::new(),
            started: Instant::now(),
            last_hello: Instant::now(),
            last_heard: None,
            quiet: false,
            protocol_mismatch: None,
            last_tick: Instant::now(),
            drag: Drag::None,
            axis_bounds: Rc::new(Cell::new(Bounds::default())),
            tiled: false,
            timeline_is_default,
            status,
            gates: GateLog::default(),
            mod_dirty: true,
            mod_editor: None,
            depth_slider: None,
            source_sliders: BTreeMap::new(),
            learning: None,
            held_keys: BTreeSet::new(),
        };
        this.ensure_source_sliders(cx);
        this.selected = this.timeline.cues.first().map(|c| c.id);
        if let Some(cue) = this.selected.and_then(|id| this.timeline.cue(id)) {
            this.playhead = cue.time;
        }

        cx.spawn_in(window, async move |this, cx| {
            loop {
                cx.background_executor().timer(TICK).await;
                let alive = this
                    .update_in(cx, |this, window, cx| this.tick(window, cx))
                    .is_ok();
                if !alive {
                    break;
                }
            }
        })
        .detach();

        this
    }

    fn mode(&self) -> Mode {
        if self.playing || self.force_live {
            return Mode::Live;
        }
        match self.selected.and_then(|id| self.timeline.cue(id)) {
            Some(cue) if (cue.time - self.playhead).abs() < 1e-3 => Mode::Edit(cue.id),
            _ => Mode::Live,
        }
    }

    // ---- modulation -------------------------------------------------------

    fn mod_ctx(&self) -> ModContext {
        ModContext {
            time: self.playhead,
            ..Default::default()
        }
    }

    /// Offsets the renderer is adding right now, computed from the same
    /// definition and clock so the ghost markers match the picture.
    fn offsets(&self) -> Vec<(ParamPath, f32)> {
        self.timeline
            .modulation
            .offsets(&self.mod_ctx(), &self.gates)
    }

    fn send_modulation(&mut self) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(&Message::Modulation {
                modulation: self.timeline.modulation.clone(),
            });
        }
        self.mod_dirty = false;
    }

    fn resend_gates(&self) {
        let Some(sender) = &self.sender else { return };
        for event in self.gates.events() {
            let _ = sender.send(&Message::Gate {
                event: event.clone(),
            });
        }
    }

    fn gate(&mut self, trigger: &str, on: bool, cx: &mut Context<Self>) {
        let event = GateEvent {
            trigger: trigger.to_owned(),
            on,
            time: self.playhead,
        };
        self.gates.push(event.clone());
        if let Some(sender) = &self.sender {
            let _ = sender.send(&Message::Gate { event });
        }
        cx.notify();
    }

    fn add_source(&mut self, envelope: bool, cx: &mut Context<Self>) {
        let source = if envelope {
            let id = self.timeline.modulation.next_id("env");
            let trigger = id.clone();
            ModSource::envelope(&id, &trigger)
        } else {
            let id = self.timeline.modulation.next_id("lfo");
            ModSource::lfo(&id, 0.25, LfoShape::Sine)
        };
        self.timeline.modulation.sources.push(source);
        self.ensure_source_sliders(cx);
        self.mod_dirty = true;
        cx.notify();
    }

    fn remove_source(&mut self, id: &str, cx: &mut Context<Self>) {
        self.timeline.modulation.remove_source(id);
        self.source_sliders.remove(id);
        let trigger_keys: Vec<String> = self
            .timeline
            .keys
            .iter()
            .filter(|(_, a)| matches!(a, KeyAction::Trigger { trigger } if trigger == id))
            .map(|(k, _)| k.clone())
            .collect();
        for k in trigger_keys {
            self.timeline.keys.remove(&k);
        }
        self.mod_dirty = true;
        cx.notify();
    }

    fn set_shape(&mut self, id: &str, shape: LfoShape, cx: &mut Context<Self>) {
        if let Some(source) = self.timeline.modulation.source_mut(id)
            && let SourceKind::Lfo { shape: s, .. } = &mut source.kind
        {
            *s = shape;
            self.mod_dirty = true;
        }
        cx.notify();
    }

    /// Sliders for every source that lacks them: LFO `[rate, phase]`,
    /// envelope `[attack, decay, sustain, release]`.
    fn ensure_source_sliders(&mut self, cx: &mut Context<Self>) {
        let sources = self.timeline.modulation.sources.clone();
        for source in sources {
            if self.source_sliders.contains_key(&source.id) {
                continue;
            }
            let specs: Vec<(f32, f32, f32)> = match &source.kind {
                SourceKind::Lfo { rate_hz, phase, .. } => {
                    vec![(0.02, 4.0, *rate_hz), (0.0, 1.0, *phase)]
                }
                SourceKind::Envelope { adsr, .. } => vec![
                    (0.0, 2.0, adsr.attack),
                    (0.0, 3.0, adsr.decay),
                    (0.0, 1.0, adsr.sustain),
                    (0.0, 4.0, adsr.release),
                ],
                _ => Vec::new(),
            };
            let mut sliders = Vec::new();
            let mut subs = Vec::new();
            for (index, (min, max, value)) in specs.into_iter().enumerate() {
                let slider = cx.new(|_| {
                    SliderState::new()
                        .min(min)
                        .max(max)
                        .step((max - min) / 400.0)
                        .default_value(value)
                });
                let id = source.id.clone();
                subs.push(
                    cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
                        if let SliderEvent::Change(v) = event {
                            this.set_source_field(&id, index, v.start(), cx);
                        }
                    }),
                );
                sliders.push(slider);
            }
            self.source_sliders
                .insert(source.id.clone(), (sliders, subs));
        }
    }

    fn set_source_field(&mut self, id: &str, index: usize, value: f32, cx: &mut Context<Self>) {
        if let Some(source) = self.timeline.modulation.source_mut(id) {
            match &mut source.kind {
                SourceKind::Lfo { rate_hz, phase, .. } => match index {
                    0 => *rate_hz = value,
                    _ => *phase = value,
                },
                SourceKind::Envelope { adsr, .. } => match index {
                    0 => adsr.attack = value,
                    1 => adsr.decay = value,
                    2 => adsr.sustain = value,
                    _ => adsr.release = value,
                },
                _ => {}
            }
            self.mod_dirty = true;
        }
        cx.notify();
    }

    /// Bipolar depth range for a parameter: its whole span either way.
    fn depth_span(desc: &ParamDescriptor) -> Option<f32> {
        match &desc.ty {
            ParamType::Float { min, max } => Some((max - min).abs().max(1e-6)),
            ParamType::Int { min, max } => Some(((max - min).abs() as f32).max(1.0)),
            _ => None,
        }
    }

    fn toggle_mod_editor(&mut self, path: &ParamPath, cx: &mut Context<Self>) {
        if self.mod_editor.as_ref() == Some(path) {
            self.mod_editor = None;
            self.depth_slider = None;
            cx.notify();
            return;
        }
        let Some(desc) = self
            .params
            .iter()
            .find(|c| &c.desc.path == path)
            .map(|c| c.desc.clone())
        else {
            return;
        };
        let Some(span) = Self::depth_span(&desc) else {
            return;
        };
        let depth = self
            .timeline
            .modulation
            .assignment(path)
            .map(|a| a.depth)
            .unwrap_or(0.0);
        let slider = cx.new(|_| {
            SliderState::new()
                .min(-span)
                .max(span)
                .step(span / 250.0)
                .default_value(depth)
        });
        let p = path.clone();
        let sub = cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(v) = event {
                this.set_depth(&p, v.start(), cx);
            }
        });
        self.depth_slider = Some((path.clone(), slider, sub));
        self.mod_editor = Some(path.clone());
        cx.notify();
    }

    fn set_depth(&mut self, path: &ParamPath, depth: f32, cx: &mut Context<Self>) {
        let source = self
            .timeline
            .modulation
            .assignment(path)
            .map(|a| a.source.clone())
            .or_else(|| {
                self.timeline
                    .modulation
                    .sources
                    .first()
                    .map(|s| s.id.clone())
            });
        if let Some(source) = source {
            self.timeline.modulation.assign(path, Some(&source), depth);
            self.mod_dirty = true;
        }
        cx.notify();
    }

    fn assign_source(&mut self, path: &ParamPath, source: Option<String>, cx: &mut Context<Self>) {
        let existing = self.timeline.modulation.assignment(path).map(|a| a.depth);
        let depth = existing.unwrap_or_else(|| {
            self.params
                .iter()
                .find(|c| &c.desc.path == path)
                .and_then(|c| Self::depth_span(&c.desc))
                .map(|span| span * 0.25)
                .unwrap_or(0.0)
        });
        self.timeline
            .modulation
            .assign(path, source.as_deref(), depth);
        self.mod_dirty = true;
        cx.notify();
    }

    fn key_for_action(&self, wanted: &KeyAction) -> Option<&str> {
        self.timeline
            .keys
            .iter()
            .find(|(_, a)| *a == wanted)
            .map(|(k, _)| k.as_str())
    }

    fn key_down(&mut self, ev: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let name = ev.keystroke.unparse();
        if let Some(target) = self.learning.take() {
            if RESERVED_KEYS.contains(&name.as_str()) {
                self.status = format!("`{name}` is a transport key; pick another");
                self.learning = Some(target);
            } else {
                let action = match &target {
                    LearnTarget::Trigger(t) => KeyAction::Trigger { trigger: t.clone() },
                    LearnTarget::Cue(id) => KeyAction::Cue { id: *id },
                };
                self.timeline.keys.retain(|_, a| a != &action);
                self.timeline.keys.insert(name.clone(), action);
                self.status = format!("bound `{name}`");
            }
            cx.notify();
            return;
        }
        if ev.is_held || self.held_keys.contains(&name) {
            return;
        }
        let Some(action) = self.timeline.keys.get(&name).cloned() else {
            return;
        };
        self.held_keys.insert(name);
        match action {
            KeyAction::Trigger { trigger } => self.gate(&trigger, true, cx),
            KeyAction::Cue { id } => self.go_to_cue(id, window, cx),
        }
    }

    fn key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        let name = ev.keystroke.unparse();
        if !self.held_keys.remove(&name) {
            return;
        }
        if let Some(KeyAction::Trigger { trigger }) = self.timeline.keys.get(&name).cloned() {
            self.gate(&trigger, false, cx);
        }
    }

    // ---- parameters -------------------------------------------------------

    fn install_params(
        &mut self,
        descriptors: Vec<ParamDescriptor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let values = self.effective_values();
        self.params.clear();
        for desc in descriptors {
            let current = values
                .get(&desc.path)
                .cloned()
                .unwrap_or_else(|| desc.value.clone());
            // (min, max, step, value) per slider component.
            let ranges: Vec<(f32, f32, f32, f32)> = match (&desc.ty, &current) {
                (ParamType::Float { min, max }, v) => {
                    vec![(
                        *min,
                        *max,
                        (max - min).abs().max(1e-6) / 1000.0,
                        v.as_float().unwrap_or(*min),
                    )]
                }
                (ParamType::Int { min, max }, v) => {
                    vec![(
                        *min as f32,
                        *max as f32,
                        1.0,
                        v.as_int().unwrap_or(*min) as f32,
                    )]
                }
                (ParamType::Vec2 { min, max }, v) => {
                    let xy = v.as_vec2().unwrap_or([0.0; 2]);
                    let step = (max - min).abs().max(1e-6) / 1000.0;
                    vec![(*min, *max, step, xy[0]), (*min, *max, step, xy[1])]
                }
                (ParamType::Color, v) => {
                    let c = v.as_color().unwrap_or([1.0; 4]);
                    vec![
                        (0.0, 1.0, 0.001, c[0]),
                        (0.0, 1.0, 0.001, c[1]),
                        (0.0, 1.0, 0.001, c[2]),
                    ]
                }
                _ => Vec::new(),
            };
            let mut sliders = Vec::new();
            let mut subscriptions = Vec::new();
            for (component, (min, max, step, value)) in ranges.into_iter().enumerate() {
                let slider = cx.new(|_| {
                    SliderState::new()
                        .min(min)
                        .max(max)
                        .step(step)
                        .default_value(value)
                });
                let path = desc.path.clone();
                subscriptions.push(cx.subscribe(
                    &slider,
                    move |this, _, event: &SliderEvent, cx| {
                        if let SliderEvent::Change(value) = event {
                            this.on_component(&path, component, value.start(), cx);
                        }
                    },
                ));
                sliders.push(slider);
            }
            self.params.push(ParamControl {
                desc,
                sliders,
                _subscriptions: subscriptions,
            });
        }
        self.sync_sliders(window, cx);
        cx.notify();
    }

    /// A slider moved: one component of a parameter changed.
    fn on_component(
        &mut self,
        path: &ParamPath,
        component: usize,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        let control = self.params.iter().find(|c| &c.desc.path == path);
        let current = self
            .effective_values()
            .get(path)
            .cloned()
            .or_else(|| control.map(|c| c.desc.value.clone()));
        let Some(current) = current else { return };
        let next = match (control.map(|c| &c.desc.ty), current) {
            (Some(ParamType::Int { .. }), _) => ParamValue::Int(value.round() as i32),
            (_, ParamValue::Float(_)) => ParamValue::Float(value),
            (_, ParamValue::Int(_)) => ParamValue::Int(value.round() as i32),
            (_, ParamValue::Vec2(mut v)) => {
                if let Some(slot) = v.get_mut(component) {
                    *slot = value;
                }
                ParamValue::Vec2(v)
            }
            (_, ParamValue::Color(mut c)) => {
                if let Some(slot) = c.get_mut(component) {
                    *slot = value;
                }
                ParamValue::Color(c)
            }
            (_, other) => other,
        };
        self.set_value(path, next, cx);
    }

    /// Apply a control change according to the current mode.
    fn set_value(&mut self, path: &ParamPath, value: ParamValue, cx: &mut Context<Self>) {
        match self.mode() {
            Mode::Edit(id) => {
                if let Some(cue) = self.timeline.cue_mut(id) {
                    cue.values.insert(path.clone(), value);
                }
                // The cue now carries the value; a stale override would hide it.
                self.overrides.remove(path);
            }
            Mode::Live => {
                self.overrides.insert(path.clone(), value);
            }
        }
        self.push_values();
        cx.notify();
    }

    /// Double-click: back to the declared default (into the cue when editing).
    fn reset_to_default(&mut self, path: &ParamPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(default) = self
            .params
            .iter()
            .find(|c| &c.desc.path == path)
            .map(|c| c.desc.default.clone())
        else {
            return;
        };
        self.set_value(path, default, cx);
        self.sync_sliders(window, cx);
    }

    fn release_override(&mut self, path: &ParamPath, window: &mut Window, cx: &mut Context<Self>) {
        self.overrides.remove(path);
        if !self.timeline.evaluate(self.playhead).contains_key(path)
            && let Some(sender) = &self.sender
        {
            let _ = sender.send(&Message::ClearParam { path: path.clone() });
            self.sent.remove(path);
        }
        self.push_values();
        self.sync_sliders(window, cx);
        cx.notify();
    }

    fn release_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<_> = self.overrides.keys().cloned().collect();
        for path in paths {
            self.release_override(&path, window, cx);
        }
    }

    /// Timeline values with manual overrides applied.
    fn effective_values(&self) -> BTreeMap<ParamPath, ParamValue> {
        let mut values = self.timeline.evaluate(self.playhead);
        for (path, value) in &self.overrides {
            values.insert(path.clone(), value.clone());
        }
        values
    }

    /// Send whatever changed since the last push.
    fn push_values(&mut self) {
        let Some(sender) = &self.sender else { return };
        let values = self.effective_values();
        let changed: Vec<(ParamPath, ParamValue)> = values
            .iter()
            .filter(|(path, value)| self.sent.get(*path) != Some(*value))
            .map(|(path, value)| (path.clone(), value.clone()))
            .collect();
        if changed.is_empty() {
            return;
        }
        for chunk in changed.chunks(32) {
            let _ = sender.send(&Message::SetParams {
                values: chunk.to_vec(),
            });
        }
        for (path, value) in changed {
            self.sent.insert(path, value);
        }
    }

    fn sync_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = self.effective_values();
        for control in &self.params {
            let Some(value) = values.get(&control.desc.path) else {
                continue;
            };
            let components: Vec<f32> = match value {
                ParamValue::Float(v) => vec![*v],
                ParamValue::Int(v) => vec![*v as f32],
                ParamValue::Vec2(v) => v.to_vec(),
                ParamValue::Color(c) => c[..3].to_vec(),
                _ => Vec::new(),
            };
            for (slider, component) in control.sliders.iter().zip(components) {
                slider.update(cx, |slider, cx| {
                    if (slider.value().start() - component).abs() > 1e-6 {
                        slider.set_value(component, window, cx);
                    }
                });
            }
        }
    }

    // ---- transport ---------------------------------------------------------

    fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32();
        self.last_tick = now;

        self.poll_network(window, cx);

        if self.playing {
            let t = self.playhead + dt;
            if !self.timeline.looping && t >= self.timeline.duration {
                self.playhead = self.timeline.duration;
                self.playing = false;
            } else {
                self.playhead = self.timeline.wrap_time(t);
            }
        }

        if self.playing || self.drag != Drag::None {
            self.sync_sliders(window, cx);
        }
        self.push_values();
        if self.mod_dirty {
            self.send_modulation();
        }
        // The renderer's clock follows this. Paused → frozen picture.
        if let Some(sender) = &self.sender {
            let _ = sender.send(&Message::Transport {
                time: self.playhead,
                playing: self.playing,
            });
        }
        cx.notify();
    }

    fn poll_network(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut installed = None;
        let messages: Vec<Message> = match &mut self.sender {
            Some(sender) => sender.poll(),
            None => Vec::new(),
        };
        for msg in messages {
            self.last_heard = Some(Instant::now());
            if self.quiet {
                self.quiet = false;
                if let Some(sender) = &self.sender {
                    let _ = sender.send(&Message::hello("zygote-timeline"));
                }
                self.last_hello = Instant::now();
            }
            match msg {
                Message::Structure {
                    protocol,
                    structure,
                } => {
                    self.note_protocol(protocol);
                    self.graph = Some(structure);
                    self.graph_fit_pending = true;
                }
                Message::Pong { protocol } => self.note_protocol(protocol),
                Message::Describe {
                    protocol,
                    graph,
                    chunk,
                    chunks,
                    params,
                } => {
                    self.note_protocol(protocol);
                    if chunk == 0 {
                        self.describe_buffer.clear();
                    }
                    self.describe_buffer.extend(params);
                    if chunk + 1 == chunks {
                        let was_described = self.described;
                        self.described = true;
                        self.status = format!(
                            "connected · graph `{graph}` · {} parameters{}",
                            self.describe_buffer.len(),
                            match self.protocol_mismatch {
                                Some(p) => format!(
                                    " · protocol mismatch (renderer {p}, ui {PROTOCOL_VERSION})"
                                ),
                                None => String::new(),
                            }
                        );
                        let fresh = std::mem::take(&mut self.describe_buffer);
                        let same = was_described
                            && self.params.len() == fresh.len()
                            && self
                                .params
                                .iter()
                                .zip(&fresh)
                                .all(|(c, d)| c.desc.path == d.path && c.desc.ty == d.ty);
                        installed = Some((fresh, same));
                    }
                }
                _ => {}
            }
        }
        if let Some((descriptors, same)) = installed {
            if self.timeline_is_default {
                let fits = self
                    .timeline
                    .cues
                    .iter()
                    .flat_map(|c| c.values.keys())
                    .any(|path| descriptors.iter().any(|d| &d.path == path));
                if !fits {
                    // Start from one cue holding the graph's defaults, parked on it.
                    let mut timeline = Timeline::new(8.0);
                    let values = descriptors
                        .iter()
                        .map(|d| (d.path.clone(), d.value.clone()))
                        .collect();
                    let id = timeline.add_cue(0.0, Transition::Cut, values);
                    self.timeline = timeline;
                    self.selected = Some(id);
                    self.playhead = 0.0;
                    self.overrides.clear();
                    self.status =
                        "new timeline: cue 1 holds the graph defaults; move sliders to edit it"
                            .to_owned();
                }
                self.timeline_is_default = false;
            }
            if !same {
                self.install_params(descriptors, window, cx);
            }
            self.sent.clear();
            self.push_values();
            self.send_modulation();
            self.resend_gates();
        } else if !self.described
            && self.params.is_empty()
            && self.started.elapsed() > DESCRIBE_TIMEOUT
        {
            self.status = format!(
                "{} · renderer not answering, using built-in first-pass parameter list",
                self.status
            );
            let library = NodeLibrary::builtin();
            let fallback = Graph::first_pass();
            self.install_params(fallback.describe_params(&library), window, cx);
            self.graph = Some(fallback.structure(&library));
            self.graph_fit_pending = true;
            self.described = true;
        }

        if self.last_hello.elapsed() >= HELLO_INTERVAL
            && let Some(sender) = &self.sender
        {
            let msg = if self.described && !self.quiet {
                Message::Ping
            } else {
                Message::hello("zygote-timeline")
            };
            let _ = sender.send(&msg);
            self.last_hello = Instant::now();
        }
        let silent = self
            .last_heard
            .is_some_and(|t| t.elapsed() > HELLO_INTERVAL * 3);
        if silent && !self.quiet {
            self.quiet = true;
            self.sent.clear();
            self.status = "renderer went quiet · waiting for it to come back".to_owned();
            cx.notify();
        }
    }

    fn note_protocol(&mut self, protocol: u32) {
        self.protocol_mismatch = (protocol != PROTOCOL_VERSION).then_some(protocol);
    }

    fn toggle_play(&mut self, cx: &mut Context<Self>) {
        self.playing = !self.playing;
        cx.notify();
    }

    fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.playing = false;
        self.seek(0.0, window, cx);
        // Park on the cue at 0 if there is one, so edits go into it.
        if let Some(cue) = self.timeline.cues.iter().find(|c| c.time.abs() < 1e-3) {
            self.selected = Some(cue.id);
        }
    }

    fn seek(&mut self, time: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.playhead = time.clamp(0.0, self.timeline.duration);
        self.sync_sliders(window, cx);
        self.push_values();
        cx.notify();
    }

    /// Select a cue and park the playhead on it (edit mode when stopped).
    fn go_to_cue(&mut self, id: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(time) = self.timeline.cue(id).map(|c| c.time) else {
            return;
        };
        self.selected = Some(id);
        self.seek(time, window, cx);
    }

    fn step_cue(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let t = self.playhead;
        let target = if forward {
            self.timeline.cues.iter().find(|c| c.time > t + 1e-3)
        } else {
            self.timeline.cues.iter().rev().find(|c| c.time < t - 1e-3)
        };
        if let Some(cue) = target.map(|c| c.id) {
            self.go_to_cue(cue, window, cx);
        }
    }

    // ---- cues -------------------------------------------------------------

    /// Snapshot everything (cues + live overrides) into a new cue at the
    /// playhead; the overrides are absorbed by it.
    fn add_cue(&mut self, cx: &mut Context<Self>) {
        if let Some(existing) = self
            .timeline
            .cues
            .iter()
            .find(|c| (c.time - self.playhead).abs() < 1e-3)
        {
            let id = existing.id;
            self.selected = Some(id);
            self.status = format!("already on cue {id}; slider changes edit it");
            cx.notify();
            return;
        }
        let values = self.effective_values();
        let id = self
            .timeline
            .add_cue(self.playhead, Transition::Interpolate, values);
        self.overrides.clear();
        self.selected = Some(id);
        self.status = format!("added cue {id} at {:.2}s", self.playhead);
        self.push_values();
        cx.notify();
    }

    /// Bake current live values into the selected cue.
    fn bake_into_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let values = self.effective_values();
        if let Some(cue) = self.timeline.cue_mut(id) {
            cue.values = values;
            self.overrides.clear();
            self.status = format!("baked live values into cue {id}");
        }
        self.push_values();
        cx.notify();
    }

    fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        self.timeline.remove_cue(id);
        self.selected = self.timeline.cues.first().map(|c| c.id);
        self.status = format!("deleted cue {id}");
        cx.notify();
    }

    fn toggle_transition(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        if let Some(cue) = self.timeline.cue_mut(id) {
            cue.transition = cue.transition.toggled();
        }
        cx.notify();
    }

    fn adjust_duration(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.timeline.duration = (self.timeline.duration + delta).clamp(1.0, 600.0);
        for cue in &mut self.timeline.cues {
            cue.time = cue.time.min(self.timeline.duration);
        }
        self.playhead = self.playhead.min(self.timeline.duration);
        cx.notify();
    }

    fn save(&mut self, cx: &mut Context<Self>) {
        match self
            .timeline
            .to_json()
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&self.file, json).map_err(|e| e.to_string()))
        {
            Ok(()) => self.status = format!("saved {}", self.file.display()),
            Err(e) => self.status = format!("save failed: {e}"),
        }
        cx.notify();
    }

    fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match std::fs::read_to_string(&self.file)
            .map_err(|e| e.to_string())
            .and_then(|json| Timeline::from_json(&json).map_err(|e| e.to_string()))
        {
            Ok(timeline) => {
                self.timeline = timeline;
                self.source_sliders.clear();
                self.ensure_source_sliders(cx);
                self.mod_dirty = true;
                self.selected = self.timeline.cues.first().map(|c| c.id);
                self.status = format!("loaded {}", self.file.display());
                self.seek(self.playhead, window, cx);
            }
            Err(e) => self.status = format!("load failed: {e}"),
        }
        cx.notify();
    }

    // ---- window arrangement ------------------------------------------------

    /// Put the output window to the right of this one, 16:9, on the same display.
    fn tile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sender) = &self.sender else { return };
        let ui = window.bounds();
        let display = window.display(cx).map(|d| d.bounds()).unwrap_or(ui);
        let scale = window.scale_factor();
        let gap = px(12.);
        let x = ui.origin.x + ui.size.width + gap;
        let available_w = (display.origin.x + display.size.width - x - gap).max(px(320.));
        let available_h = (display.size.height - px(80.)).max(px(180.));
        let mut w = available_w;
        let mut h = w * 9.0 / 16.0;
        if h > available_h {
            h = available_h;
            w = h * 16.0 / 9.0;
        }
        let f = |v: Pixels| -> f32 { v.into() };
        // Bevy positions windows in physical pixels and sizes them in logical pixels.
        let bounds = OutputBounds {
            x: (f(x) * scale) as i32,
            y: (f(ui.origin.y) * scale) as i32,
            width: f(w) as u32,
            height: f(h) as u32,
        };
        let _ = sender.send(&Message::Arrange {
            bounds: Some(bounds),
        });
        self.tiled = true;
        self.status = format!(
            "output tiled at {}x{} next to this window",
            bounds.width, bounds.height
        );
        cx.notify();
    }

    fn release_output(&mut self, cx: &mut Context<Self>) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(&Message::Arrange { bounds: None });
        }
        self.tiled = false;
        self.status = "output window released".to_owned();
        cx.notify();
    }

    // ---- timeline axis -----------------------------------------------------

    fn time_at(&self, x: Pixels) -> f32 {
        let bounds = self.axis_bounds.get();
        let width: f32 = bounds.size.width.into();
        if width <= 0.0 {
            return 0.0;
        }
        let local: f32 = (x - bounds.origin.x).into();
        (local / width).clamp(0.0, 1.0) * self.timeline.duration
    }

    fn axis_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = Drag::Playhead;
        let t = self.time_at(ev.position.x);
        self.seek(t, window, cx);
    }

    fn cue_mouse_down(&mut self, id: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.drag = Drag::Cue(id);
        self.go_to_cue(id, window, cx);
        cx.stop_propagation();
    }

    fn axis_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t = self.time_at(ev.position.x);
        match self.drag {
            Drag::None => {}
            Drag::Playhead => self.seek(t, window, cx),
            Drag::Cue(id) => {
                self.timeline.move_cue(id, t);
                self.seek(t, window, cx);
            }
        }
    }

    fn axis_mouse_up(&mut self, cx: &mut Context<Self>) {
        self.drag = Drag::None;
        cx.notify();
    }

    // ---- graph viewport ------------------------------------------------------

    fn graph_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        self.graph_drag = Some((ev.position, self.graph_pan));
        cx.notify();
    }

    fn graph_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some((start, pan)) = self.graph_drag {
            let dx: f32 = (ev.position.x - start.x).into();
            let dy: f32 = (ev.position.y - start.y).into();
            self.graph_pan = point(pan.x + dx, pan.y + dy);
            cx.notify();
        }
    }

    fn graph_mouse_up(&mut self, cx: &mut Context<Self>) {
        self.graph_drag = None;
        cx.notify();
    }

    fn graph_scroll(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = ev.delta.pixel_delta(px(24.));
        let dx: f32 = delta.x.into();
        let dy: f32 = delta.y.into();
        if ev.modifiers.shift {
            self.graph_pan = point(self.graph_pan.x + dy, self.graph_pan.y + dx);
        } else {
            self.graph_pan = point(self.graph_pan.x + dx + dy, self.graph_pan.y);
        }
        cx.notify();
    }

    fn graph_fit(&mut self, content: (f32, f32), cx: &mut Context<Self>) {
        let viewport = self.graph_viewport.get();
        let vw: f32 = viewport.size.width.into();
        let vh: f32 = viewport.size.height.into();
        if vw <= 0.0 {
            return;
        }
        let x = if content.0 <= vw {
            (vw - content.0) / 2.0
        } else {
            0.0
        };
        let y = ((vh - content.1) / 2.0).max(0.0);
        self.graph_pan = point(x, y);
        self.graph_fit_pending = false;
        cx.notify();
    }

    // ---- rendering ---------------------------------------------------------

    fn render_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mut rail = v_flex()
            .w(px(RAIL_WIDTH))
            .flex_shrink_0()
            .h_full()
            .gap_1()
            .pr_3()
            .border_r_1()
            .border_color(theme.border);
        rail = rail.child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .pb_1()
                .child("NODES"),
        );
        let all_selected = self.selected_node.is_none();
        rail = rail.child(
            div()
                .id("node-all")
                .px_2()
                .py_1()
                .rounded_md()
                .text_sm()
                .cursor_pointer()
                .when(all_selected, |d| d.bg(theme.primary.opacity(0.12)))
                .text_color(if all_selected {
                    theme.primary
                } else {
                    theme.foreground
                })
                .child("All parameters")
                .on_click(cx.listener(|this, _, _, cx| {
                    this.selected_node = None;
                    cx.notify();
                })),
        );
        if let Some(graph) = &self.graph {
            let depths = structure_depths(graph);
            for (i, node) in graph.nodes.iter().enumerate() {
                let selected = self.selected_node.as_ref() == Some(&node.id);
                let is_output = node.id == graph.output;
                let depth = depths.get(&node.id).copied().unwrap_or(0);
                let id = node.id.clone();
                rail = rail.child(
                    v_flex()
                        .id(("node", i))
                        .px_2()
                        .py_1()
                        .rounded_md()
                        .cursor_pointer()
                        .when(selected, |d| d.bg(theme.primary.opacity(0.12)))
                        .child(
                            h_flex()
                                .gap_1()
                                .items_center()
                                .child(div().w(px(depth as f32 * 8.0)))
                                .child(
                                    div()
                                        .text_sm()
                                        .truncate()
                                        .text_color(if selected {
                                            theme.primary
                                        } else {
                                            theme.foreground
                                        })
                                        .child(node.id.to_string()),
                                )
                                .when(is_output, |d| {
                                    d.child(
                                        div().text_xs().text_color(theme.primary).child("→ out"),
                                    )
                                }),
                        )
                        .child(
                            div()
                                .pl(px(depth as f32 * 8.0))
                                .text_xs()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(node.kind.clone()),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.selected_node = Some(id.clone());
                            cx.notify();
                        })),
                );
            }
        }
        rail = rail.child(self.render_rack(cx));
        rail = rail.child(div().flex_1());
        rail = rail.child(
            Button::new("toggle-graph")
                .small()
                .label(if self.show_graph {
                    "Hide graph"
                } else {
                    "Show graph"
                })
                .on_click(cx.listener(|this, _, _, cx| {
                    this.show_graph = !this.show_graph;
                    this.graph_fit_pending = true;
                    cx.notify();
                })),
        );
        rail.into_any_element()
    }

    /// Shared modulation sources with live meters.
    fn render_rack(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let ctx = self.mod_ctx();
        let mut rack = v_flex()
            .gap_2()
            .pt_3()
            .mt_2()
            .border_t_1()
            .border_color(theme.border);
        rack = rack.child(
            h_flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .flex_1()
                        .child("MODULATION"),
                )
                .child(
                    Button::new("add-lfo")
                        .xsmall()
                        .label("+ LFO")
                        .on_click(cx.listener(|this, _, _, cx| this.add_source(false, cx))),
                )
                .child(
                    Button::new("add-env")
                        .xsmall()
                        .label("+ ENV")
                        .on_click(cx.listener(|this, _, _, cx| this.add_source(true, cx))),
                ),
        );
        for (i, source) in self.timeline.modulation.sources.iter().enumerate() {
            let value = source.sample(&ctx, &self.gates);
            let (frac, bipolar) = match source.kind {
                SourceKind::Lfo { .. } => ((value + 1.0) * 0.5, true),
                _ => (value, false),
            };
            let id = source.id.clone();
            let uses = self
                .timeline
                .modulation
                .assignments
                .iter()
                .filter(|a| a.source == source.id)
                .count();
            let mut card = v_flex()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.muted.opacity(0.25));
            let remove_id = id.clone();
            card = card.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .flex_1()
                            .truncate()
                            .text_color(theme.foreground)
                            .child(source.id.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{uses}×")),
                    )
                    .child(Button::new(("rm-source", i)).xsmall().label("✕").on_click(
                        cx.listener(move |this, _, _, cx| this.remove_source(&remove_id, cx)),
                    )),
            );
            // Live meter.
            card = card.child(
                div()
                    .relative()
                    .w_full()
                    .h(px(6.))
                    .rounded_sm()
                    .bg(theme.muted)
                    .when(bipolar, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(relative(0.5))
                                .w(px(1.))
                                .bg(theme.border),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .top(px(-2.))
                            .left(relative(frac.clamp(0.0, 1.0)))
                            .ml(px(-3.))
                            .w(px(6.))
                            .h(px(10.))
                            .rounded_sm()
                            .bg(theme.primary),
                    ),
            );
            let sliders = self
                .source_sliders
                .get(&source.id)
                .map(|(s, _)| s.clone())
                .unwrap_or_default();
            match &source.kind {
                SourceKind::Lfo { shape, .. } => {
                    let mut shapes = h_flex().gap_1();
                    for (j, candidate) in LfoShape::ALL.iter().enumerate() {
                        let id = id.clone();
                        let c = *candidate;
                        shapes = shapes.child(
                            Button::new(("shape", (i * 8 + j) as u64))
                                .xsmall()
                                .label(c.label())
                                .when(*shape == c, |b| b.primary())
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.set_shape(&id, c, cx)),
                                ),
                        );
                    }
                    card = card.child(shapes);
                    for (slider, name) in sliders.iter().zip(["rate", "phase"]) {
                        card = card.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(36.))
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(name),
                                )
                                .child(Slider::new(slider).flex_1()),
                        );
                    }
                }
                SourceKind::Envelope { trigger, .. } => {
                    let bound = self
                        .key_for_action(&KeyAction::Trigger {
                            trigger: trigger.clone(),
                        })
                        .map(|k| format!("key `{k}`"))
                        .unwrap_or_else(|| "no key".to_owned());
                    let learning = self.learning == Some(LearnTarget::Trigger(trigger.clone()));
                    let t_learn = trigger.clone();
                    let t_down = trigger.clone();
                    let t_up = trigger.clone();
                    card = card.child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .flex_1()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(bound),
                            )
                            .child(
                                Button::new(("learn", i))
                                    .xsmall()
                                    .label(if learning { "press a key…" } else { "Learn" })
                                    .when(learning, |b| b.primary())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.learning = Some(LearnTarget::Trigger(t_learn.clone()));
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id(("fire", i))
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_xs()
                                    .cursor_pointer()
                                    .child("hold")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.gate(&t_down, true, cx)
                                        }),
                                    )
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.gate(&t_up, false, cx)
                                        }),
                                    ),
                            ),
                    );
                    for (slider, name) in sliders.iter().zip(["A", "D", "S", "R"]) {
                        card = card.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(36.))
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(name),
                                )
                                .child(Slider::new(slider).flex_1()),
                        );
                    }
                }
                _ => {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(source.label()),
                    );
                }
            }
            rack = rack.child(card);
        }
        if self.timeline.modulation.sources.is_empty() {
            rack = rack.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("no sources yet · add an LFO or an envelope, then assign it from a parameter's `mod` chip"),
            );
        }
        rack.into_any_element()
    }

    fn render_transport(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mode = self.mode();
        let (mode_label, mode_color) = match mode {
            Mode::Edit(id) => (format!("editing cue {id}"), theme.primary),
            Mode::Live => (
                if self.playing {
                    "live"
                } else {
                    "live · not on a cue"
                }
                .to_owned(),
                theme.warning,
            ),
        };
        h_flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(
                Button::new("play")
                    .small()
                    .primary()
                    .label(if self.playing {
                        "Pause  ␣"
                    } else {
                        "Play  ␣"
                    })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
            )
            .child(
                Button::new("stop")
                    .small()
                    .label("Stop  esc")
                    .on_click(cx.listener(|this, _, window, cx| this.stop(window, cx))),
            )
            .child(
                Button::new("prev-cue")
                    .small()
                    .label("[ prev")
                    .on_click(cx.listener(|this, _, window, cx| this.step_cue(false, window, cx))),
            )
            .child(
                Button::new("next-cue")
                    .small()
                    .label("next ]")
                    .on_click(cx.listener(|this, _, window, cx| this.step_cue(true, window, cx))),
            )
            .child(
                Button::new("loop")
                    .small()
                    .label(if self.timeline.looping {
                        "Loop: on"
                    } else {
                        "Loop: off"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.timeline.looping = !this.timeline.looping;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .px_2()
                    .font_family("monospace")
                    .text_color(theme.foreground)
                    .child(format!(
                        "{:6.2}s / {:.0}s",
                        self.playhead, self.timeline.duration
                    )),
            )
            .child(
                Button::new("dur-")
                    .small()
                    .label("−1s")
                    .on_click(cx.listener(|this, _, _, cx| this.adjust_duration(-1.0, cx))),
            )
            .child(
                Button::new("dur+")
                    .small()
                    .label("+1s")
                    .on_click(cx.listener(|this, _, _, cx| this.adjust_duration(1.0, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .text_xs()
                    .bg(mode_color.opacity(0.12))
                    .text_color(mode_color)
                    .child(mode_label),
            )
            .child(
                Button::new("mode")
                    .small()
                    .label(if self.force_live {
                        "Edit cues  e"
                    } else {
                        "Force live  e"
                    })
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.force_live = !this.force_live;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    fn render_cue_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.and_then(|id| self.timeline.cue(id));
        let theme = cx.theme();
        h_flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(
                Button::new("add-cue")
                    .small()
                    .primary()
                    .label("Add cue here  ⏎")
                    .on_click(cx.listener(|this, _, _, cx| this.add_cue(cx))),
            )
            .child(
                Button::new("bake-cue")
                    .small()
                    .label("Bake live values into cue")
                    .disabled(selected.is_none() || self.overrides.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.bake_into_selected(cx))),
            )
            .child(
                Button::new("toggle-transition")
                    .small()
                    .label(match selected.map(|c| c.transition) {
                        Some(Transition::Cut) => "Into cue: step",
                        Some(Transition::Interpolate) => "Into cue: ramp",
                        None => "Into cue: —",
                    })
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_transition(cx))),
            )
            .child(
                Button::new("delete-cue")
                    .small()
                    .danger()
                    .label("Delete cue")
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx))),
            )
            .child(
                Button::new("learn-cue-key")
                    .small()
                    .label(match selected {
                        Some(cue) => match (
                            &self.learning,
                            self.key_for_action(&KeyAction::Cue { id: cue.id }),
                        ) {
                            (Some(LearnTarget::Cue(id)), _) if *id == cue.id => {
                                "press a key…".to_owned()
                            }
                            (_, Some(k)) => format!("Cue key: {k}"),
                            _ => "Learn cue key".to_owned(),
                        },
                        None => "Learn cue key".to_owned(),
                    })
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(id) = this.selected {
                            this.learning = Some(LearnTarget::Cue(id));
                            cx.notify();
                        }
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(match selected {
                        Some(cue) => format!(
                            "{} · {:.2}s · {} values",
                            cue.label,
                            cue.time,
                            cue.values.len()
                        ),
                        None => "no cue selected".to_owned(),
                    }),
            )
            .child(
                Button::new("save")
                    .small()
                    .label("Save")
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child(
                Button::new("load")
                    .small()
                    .label("Load")
                    .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
            )
            .into_any_element()
    }

    /// Time axis: tick row, cue chips on a lane, segment lane showing ramps
    /// and steps between cues, and the playhead across everything.
    fn render_axis(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let duration = self.timeline.duration.max(1e-3);
        let bounds_cell = self.axis_bounds.clone();
        let primary = theme.primary;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let danger = theme.danger;
        let selected = self.selected;
        let playhead = self.playhead;

        // Geometry (pixels from the top of the axis).
        let tick_h = 18.0;
        let chip_top = 24.0;
        let chip_h = 22.0;
        let seg_top = chip_top + chip_h + 10.0;
        let seg_bottom = AXIS_HEIGHT - 8.0;

        let cues: Vec<(f32, Transition)> = self
            .timeline
            .cues
            .iter()
            .map(|c| (c.time, c.transition))
            .collect();
        let segment_color = muted.opacity(0.8);

        let mut axis = div()
            .relative()
            .w_full()
            .h(px(AXIS_HEIGHT))
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(theme.muted.opacity(0.35))
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    move |bounds, _, window, _| {
                        let w: f32 = bounds.size.width.into();
                        let ox = bounds.origin.x;
                        let oy = bounds.origin.y;
                        let x_at = |t: f32| ox + px(w * (t / duration).clamp(0.0, 1.0));

                        // Segment lane: hold at the bottom, ramp up to the next cue
                        // when it interpolates, jump when it cuts.
                        if let Some(first) = cues.first() {
                            let mut path = PathBuilder::stroke(px(1.5));
                            path.move_to(point(ox, oy + px(seg_bottom)));
                            path.line_to(point(x_at(first.0), oy + px(seg_bottom)));
                            for pair in cues.windows(2) {
                                let (a, (b_time, b_transition)) = (pair[0].0, pair[1]);
                                match b_transition {
                                    Transition::Interpolate => {
                                        path.move_to(point(x_at(a), oy + px(seg_bottom)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_top)));
                                        path.move_to(point(x_at(b_time), oy + px(seg_top)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_bottom)));
                                    }
                                    Transition::Cut => {
                                        path.move_to(point(x_at(a), oy + px(seg_bottom)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_bottom)));
                                        path.move_to(point(x_at(b_time), oy + px(seg_bottom)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_top)));
                                        path.move_to(point(x_at(b_time), oy + px(seg_top)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_bottom)));
                                    }
                                }
                            }
                            if let Some(last) = cues.last() {
                                path.move_to(point(x_at(last.0), oy + px(seg_bottom)));
                                path.line_to(point(ox + px(w), oy + px(seg_bottom)));
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, segment_color);
                            }
                        }

                        // Playhead: line across the axis with a head at the top.
                        let px_x = x_at(playhead);
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(px_x - px(1.), oy),
                                size: size(px(2.), px(AXIS_HEIGHT)),
                            },
                            danger,
                        ));
                        let mut head = PathBuilder::fill();
                        head.move_to(point(px_x - px(6.), oy));
                        head.line_to(point(px_x + px(6.), oy));
                        head.line_to(point(px_x, oy + px(8.)));
                        head.close();
                        if let Ok(head) = head.build() {
                            window.paint_path(head, danger);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            );

        // Ticks.
        let step = if duration > 60.0 {
            10.0
        } else if duration > 20.0 {
            5.0
        } else {
            1.0
        };
        let mut t = 0.0;
        while t <= duration + 1e-3 {
            let frac = t / duration;
            axis = axis
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(relative(frac))
                        .w(px(1.))
                        .h(px(6.))
                        .bg(muted.opacity(0.6)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .left(relative(frac))
                        .ml(px(3.))
                        .text_xs()
                        .text_color(muted)
                        .child(format!("{t:.0}")),
                );
            t += step;
        }
        let _ = tick_h;

        // Cue chips.
        for cue in &self.timeline.cues {
            let frac = (cue.time / duration).clamp(0.0, 1.0);
            let is_selected = selected == Some(cue.id);
            let id = cue.id;
            let glyph = match cue.transition {
                Transition::Cut => "⌐",
                Transition::Interpolate => "⟋",
            };
            axis = axis.child(
                h_flex()
                    .id(("cue", id as usize))
                    .absolute()
                    .top(px(chip_top))
                    .left(relative(frac))
                    .ml(px(-6.))
                    .h(px(chip_h))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .border_color(if is_selected { primary } else { border })
                    .bg(if is_selected {
                        primary
                    } else {
                        theme.background
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_selected {
                                theme.primary_foreground
                            } else {
                                muted
                            })
                            .child(glyph),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_selected {
                                theme.primary_foreground
                            } else {
                                theme.foreground
                            })
                            .child(cue.label.clone()),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| this.cue_mouse_down(id, window, cx)),
                    ),
            );
        }

        axis.id("axis")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.axis_mouse_down(ev, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                this.axis_mouse_move(ev, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.axis_mouse_up(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.axis_mouse_up(cx)),
            )
            .into_any_element()
    }

    /// Read-only picture of the renderer's node graph, pannable.
    fn render_graph(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(graph) = self.graph.clone() else {
            return div()
                .h(px(GRAPH_VIEW_H))
                .flex()
                .items_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("waiting for the renderer to send its graph…")
                .into_any_element();
        };
        let depths = structure_depths(&graph);
        let mut rows_in_col: BTreeMap<usize, usize> = BTreeMap::new();
        let mut placed: BTreeMap<NodeId, (usize, usize)> = BTreeMap::new();
        for node in &graph.nodes {
            let col = depths.get(&node.id).copied().unwrap_or(0);
            let row = rows_in_col.entry(col).or_insert(0);
            placed.insert(node.id.clone(), (col, *row));
            *row += 1;
        }
        let cols = rows_in_col.keys().max().map(|c| c + 1).unwrap_or(1);
        let rows = rows_in_col.values().copied().max().unwrap_or(1);
        let width = GRAPH_PAD * 2.0 + (cols as f32 + 1.0) * NODE_W + cols as f32 * COL_GAP;
        let height =
            GRAPH_PAD * 2.0 + rows as f32 * NODE_H + (rows as f32 - 1.0).max(0.0) * ROW_GAP;
        if self.graph_fit_pending {
            self.graph_fit((width, height), cx);
        }
        let pan = self.graph_pan;
        let theme = cx.theme();
        let pos = |col: usize, row: usize| -> (f32, f32) {
            (
                GRAPH_PAD + col as f32 * (NODE_W + COL_GAP),
                GRAPH_PAD + row as f32 * (NODE_H + ROW_GAP),
            )
        };

        let mut edges: Vec<((f32, f32), (f32, f32))> = Vec::new();
        for node in &graph.nodes {
            let Some(&(col, row)) = placed.get(&node.id) else {
                continue;
            };
            let (x, y) = pos(col, row);
            let slots = node.inputs.len().max(1) as f32;
            for (slot, link) in node.inputs.iter().enumerate() {
                let Some(input) = &link.from else { continue };
                let Some(&(icol, irow)) = placed.get(input) else {
                    continue;
                };
                let (ix, iy) = pos(icol, irow);
                let slot_y = y + NODE_H * (slot as f32 + 1.0) / (slots + 1.0);
                edges.push(((ix + NODE_W, iy + NODE_H / 2.0), (x, slot_y)));
            }
        }
        let window_col = cols;
        if let Some(&(col, row)) = placed.get(&graph.output) {
            let (x, y) = pos(col, row);
            let (wx, wy) = pos(window_col, 0);
            edges.push(((x + NODE_W, y + NODE_H / 2.0), (wx, wy + NODE_H / 2.0)));
        }

        let edge_color = theme.muted_foreground.opacity(0.7);
        let mut view = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .flex_shrink_0()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        for ((fx, fy), (tx, ty)) in edges {
                            let from = bounds.origin + point(px(fx), px(fy));
                            let to = bounds.origin + point(px(tx), px(ty));
                            let dx = (to.x - from.x) * 0.5;
                            let mut path = PathBuilder::stroke(px(1.5));
                            path.move_to(from);
                            path.cubic_bezier_to(
                                to,
                                point(from.x + dx, from.y),
                                point(to.x - dx, to.y),
                            );
                            if let Ok(path) = path.build() {
                                window.paint_path(path, edge_color);
                            }
                            let mut head = PathBuilder::fill();
                            head.move_to(to);
                            head.line_to(point(to.x - px(7.), to.y - px(4.)));
                            head.line_to(point(to.x - px(7.), to.y + px(4.)));
                            head.close();
                            if let Ok(head) = head.build() {
                                window.paint_path(head, edge_color);
                            }
                        }
                    },
                )
                .absolute()
                .inset_0(),
            );

        for node in &graph.nodes {
            let Some(&(col, row)) = placed.get(&node.id) else {
                continue;
            };
            let (x, y) = pos(col, row);
            let is_output = node.id == graph.output;
            let is_source = node.inputs.is_empty();
            let mut slots = v_flex().justify_around().h_full().pr_1();
            for link in &node.inputs {
                let connected = link.from.is_some();
                slots = slots.child(
                    div()
                        .text_xs()
                        .text_color(if connected {
                            theme.muted_foreground
                        } else {
                            theme.muted_foreground.opacity(0.4)
                        })
                        .child(link.name.clone()),
                );
            }
            view = view.child(
                h_flex()
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(NODE_W))
                    .h(px(NODE_H))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .border_color(if is_output {
                        theme.primary
                    } else {
                        theme.border
                    })
                    .bg(if is_source {
                        theme.muted
                    } else {
                        theme.background
                    })
                    .when(!node.enabled, |d| d.opacity(0.45))
                    .when(!node.inputs.is_empty(), |d| d.child(slots))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .text_color(theme.foreground)
                                    .child(node.id.to_string()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "{}{}",
                                        node.kind,
                                        if node.feedback { " · feedback" } else { "" }
                                    )),
                            ),
                    ),
            );
        }
        let (wx, wy) = pos(window_col, 0);
        view = view.child(
            v_flex()
                .absolute()
                .left(px(wx))
                .top(px(wy))
                .w(px(NODE_W))
                .h(px(NODE_H))
                .px_2()
                .justify_center()
                .rounded_md()
                .border_1()
                .border_dashed()
                .border_color(theme.primary)
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .text_color(theme.foreground)
                        .child("output window"),
                )
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child("3D quad · perspective camera"),
                ),
        );

        let viewport_cell = self.graph_viewport.clone();
        let content = (width, height);
        div()
            .id("graph-viewport")
            .relative()
            .w_full()
            .h(px(GRAPH_VIEW_H))
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.2))
            .cursor_grab()
            .child(
                canvas(
                    move |bounds, _, _| viewport_cell.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(view.absolute().left(px(pan.x)).top(px(pan.y)))
            .child(
                h_flex()
                    .absolute()
                    .top_1()
                    .right_1()
                    .gap_1()
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!(
                                "{} nodes · drag or scroll to pan",
                                graph.nodes.len()
                            )),
                    )
                    .child(
                        Button::new("graph-fit").xsmall().label("Fit").on_click(
                            cx.listener(move |this, _, _, cx| this.graph_fit(content, cx)),
                        ),
                    ),
            )
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| this.graph_mouse_down(ev, cx)),
            )
            .on_mouse_move(
                cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.graph_mouse_move(ev, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.graph_mouse_up(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.graph_mouse_up(cx)),
            )
            .on_scroll_wheel(
                cx.listener(|this, ev: &ScrollWheelEvent, _, cx| this.graph_scroll(ev, cx)),
            )
            .into_any_element()
    }

    fn render_params(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let values = self.effective_values();
        let offsets = self.offsets();
        let mode = self.mode();
        let mut list = v_flex().gap_1().w_full();
        if self.params.is_empty() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("waiting for the renderer to describe its graph…"),
            );
        }
        let mut last_node: Option<NodeId> = None;
        for (i, control) in self.params.iter().enumerate() {
            let path = control.desc.path.clone();
            if let Some(filter) = &self.selected_node
                && filter != &path.node
            {
                continue;
            }
            if last_node.as_ref() != Some(&path.node) {
                last_node = Some(path.node.clone());
                let kind = self
                    .graph
                    .as_ref()
                    .and_then(|g| g.nodes.iter().find(|n| n.id == path.node))
                    .map(|n| n.kind.clone())
                    .unwrap_or_else(|| control.desc.node_kind.clone());
                list = list.child(
                    h_flex()
                        .gap_2()
                        .items_baseline()
                        .px_2()
                        .pt_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.foreground)
                                .child(path.node.to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(kind),
                        ),
                );
            }
            let overridden = self.overrides.contains_key(&path);
            let in_cue = matches!(mode, Mode::Edit(id) if self.timeline.cue(id).is_some_and(|c| c.values.contains_key(&path)));
            let value = values
                .get(&path)
                .cloned()
                .unwrap_or_else(|| control.desc.value.clone());
            let label_color = if overridden {
                theme.warning
            } else if in_cue {
                theme.primary
            } else {
                theme.foreground
            };

            let widget: AnyElement = match (&control.desc.ty, &value) {
                (ParamType::Bool, v) => {
                    let checked = v.as_bool().unwrap_or(false);
                    let path = path.clone();
                    Switch::new(("switch", i))
                        .checked(checked)
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.set_value(&path, ParamValue::Bool(*checked), cx)
                        }))
                        .into_any_element()
                }
                (ParamType::Choice { options }, v) => {
                    let current = v.as_choice().unwrap_or("");
                    let mut row = h_flex().gap_1().flex_wrap();
                    for (j, option) in options.iter().enumerate() {
                        let selected = option == current;
                        let path = path.clone();
                        let option_value = option.clone();
                        row = row.child(
                            Button::new(("choice", (i * 64 + j) as u64))
                                .xsmall()
                                .label(option.clone())
                                .when(selected, |b| b.primary())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_value(
                                        &path,
                                        ParamValue::Choice(option_value.clone()),
                                        cx,
                                    )
                                })),
                        );
                    }
                    row.into_any_element()
                }
                (ParamType::Color, v) => {
                    let c = v.as_color().unwrap_or([1.0; 4]);
                    let swatch = Rgba {
                        r: c[0],
                        g: c[1],
                        b: c[2],
                        a: 1.0,
                    };
                    let mut row = h_flex().gap_2().items_center().flex_1();
                    row = row.child(
                        div()
                            .w(px(22.))
                            .h(px(22.))
                            .rounded_sm()
                            .border_1()
                            .border_color(theme.border)
                            .bg(swatch),
                    );
                    for (slider, name) in control.sliders.iter().zip(["r", "g", "b"]) {
                        row = row
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(name),
                            )
                            .child(Slider::new(slider).flex_1());
                    }
                    row.into_any_element()
                }
                (ParamType::Vec2 { .. }, _) => {
                    let mut row = h_flex().gap_2().items_center().flex_1();
                    for (slider, name) in control.sliders.iter().zip(["x", "y"]) {
                        row = row
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(name),
                            )
                            .child(Slider::new(slider).flex_1());
                    }
                    row.into_any_element()
                }
                (ParamType::Float { .. } | ParamType::Int { .. }, v) => {
                    match control.sliders.first() {
                        Some(slider) => {
                            // Ghost marker: where the value actually is once modulation is added.
                            let ghost = offsets
                                .iter()
                                .find(|(p, _)| p == &path)
                                .map(|(_, offset)| {
                                    control.desc.ty.conform(&apply_offset(v, *offset))
                                })
                                .and_then(|g| {
                                    let (min, max) = match &control.desc.ty {
                                        ParamType::Float { min, max } => (*min, *max),
                                        ParamType::Int { min, max } => (*min as f32, *max as f32),
                                        _ => (0.0, 1.0),
                                    };
                                    g.as_float().map(|g| {
                                        ((g - min) / (max - min).max(1e-6)).clamp(0.0, 1.0)
                                    })
                                });
                            div()
                                .relative()
                                .flex_1()
                                .child(Slider::new(slider).w_full())
                                .when_some(ghost, |d, frac| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .top(px(-3.))
                                            .left(relative(frac))
                                            .ml(px(-1.))
                                            .w(px(2.))
                                            .h(px(22.))
                                            .rounded_sm()
                                            .bg(theme.primary.opacity(0.8)),
                                    )
                                })
                                .into_any_element()
                        }
                        None => div().into_any_element(),
                    }
                }
            };

            let reset_path = path.clone();
            let assignment = self.timeline.modulation.assignment(&path).cloned();
            let modulatable = Self::depth_span(&control.desc).is_some();
            let editor_open = self.mod_editor.as_ref() == Some(&path);
            let chip_path = path.clone();
            list = list.child(
                h_flex()
                    .id(("param-row", i))
                    .gap_2()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(overridden, |d| d.bg(theme.warning.opacity(0.08)))
                    // Capture phase so the slider underneath cannot swallow the double-click.
                    .capture_any_mouse_down(cx.listener(
                        move |this, ev: &MouseDownEvent, window, cx| {
                            if ev.click_count >= 2 {
                                this.reset_to_default(&reset_path, window, cx);
                                cx.stop_propagation();
                            }
                        },
                    ))
                    .child(
                        div()
                            .w(px(170.))
                            .pl_3()
                            .text_sm()
                            .truncate()
                            .text_color(label_color)
                            .child(path.param.clone()),
                    )
                    .child(div().flex_1().child(widget))
                    .child(
                        div()
                            .w(px(72.))
                            .text_sm()
                            .font_family("monospace")
                            .text_color(theme.muted_foreground)
                            .child(value.to_string()),
                    )
                    .child(
                        Button::new(("mod", i))
                            .xsmall()
                            .label(match &assignment {
                                Some(a) => format!("{} ±{:.2}", a.source, a.depth.abs()),
                                None => "mod".to_owned(),
                            })
                            .when(assignment.is_some() || editor_open, |b| b.primary())
                            .disabled(!modulatable)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_mod_editor(&chip_path, cx)
                            })),
                    )
                    .child(
                        Button::new(("release", i))
                            .xsmall()
                            .label(if overridden {
                                "live ✕"
                            } else if in_cue {
                                "in cue"
                            } else {
                                "default"
                            })
                            .disabled(!overridden)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.release_override(&path, window, cx)
                            })),
                    ),
            );

            if editor_open {
                let mut sources = h_flex().gap_1().items_center().flex_wrap();
                let none_path = control.desc.path.clone();
                sources = sources.child(
                    Button::new(("mod-none", i))
                        .xsmall()
                        .label("none")
                        .when(assignment.is_none(), |b| b.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.assign_source(&none_path, None, cx)
                        })),
                );
                for (j, source) in self.timeline.modulation.sources.iter().enumerate() {
                    let sp = control.desc.path.clone();
                    let sid = source.id.clone();
                    let selected = assignment.as_ref().is_some_and(|a| a.source == source.id);
                    sources = sources.child(
                        Button::new(("mod-src", (i * 64 + j) as u64))
                            .xsmall()
                            .label(source.id.clone())
                            .when(selected, |b| b.primary())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.assign_source(&sp, Some(sid.clone()), cx)
                            })),
                    );
                }
                if self.timeline.modulation.sources.is_empty() {
                    sources = sources.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("add an LFO or envelope in the rail first"),
                    );
                }
                let mut editor = h_flex()
                    .gap_2()
                    .items_center()
                    .ml(px(190.))
                    .mr_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme.primary.opacity(0.06))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("source"),
                    )
                    .child(sources);
                if let Some((p, slider, _)) = &self.depth_slider
                    && p == &control.desc.path
                {
                    editor = editor
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("depth ±"),
                        )
                        .child(Slider::new(slider).flex_1().min_w(px(160.)));
                }
                list = list.child(editor);
            }
        }
        list.into_any_element()
    }
}

impl Render for TimelineApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (background, foreground, muted) = {
            let theme = cx.theme();
            (theme.background, theme.foreground, theme.muted_foreground)
        };
        let header = h_flex()
            .items_center()
            .gap_3()
            .child(div().text_lg().child("Zygote"))
            .child(div().flex_1().min_w_0().text_xs().truncate().text_color(muted).child(
                "space play · esc stop · [ ] cues · ⏎ add cue · e live/edit · l loop · t tile · double-click resets",
            ))
            .child(
                Button::new("tile")
                    .small()
                    .label(if self.tiled { "Re-tile output" } else { "Tile output →" })
                    .disabled(self.sender.is_none())
                    .on_click(cx.listener(|this, _, window, cx| this.tile(window, cx))),
            )
            .child(
                Button::new("release-output")
                    .small()
                    .label("Pop out")
                    .disabled(!self.tiled)
                    .on_click(cx.listener(|this, _, _, cx| this.release_output(cx))),
            )
            .child(
                Button::new("release-all")
                    .small()
                    .label("Release live")
                    .disabled(self.overrides.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.release_all(window, cx))),
            );
        let rail = self.render_rail(cx);
        let transport = self.render_transport(cx);
        let graph = if self.show_graph {
            Some(self.render_graph(cx))
        } else {
            None
        };
        let axis = self.render_axis(cx);
        let cue_bar = self.render_cue_bar(cx);
        let params = self.render_params(cx);

        let mut main = v_flex().flex_1().min_w_0().gap_3().child(transport);
        if let Some(graph) = graph {
            main = main.child(graph);
        }
        main = main.child(axis).child(cue_bar).child(
            div()
                .id("params")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(params),
        );

        v_flex()
            .id("root")
            .track_focus(&self.focus_handle)
            .key_context("Zygote")
            .capture_action(cx.listener(|this, _: &TogglePlay, _, cx| {
                this.toggle_play(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Stop, window, cx| {
                if this.learning.take().is_some() {
                    cx.notify();
                } else {
                    this.stop(window, cx);
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &GoToStart, window, cx| {
                this.seek(0.0, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &PrevCue, window, cx| {
                this.step_cue(false, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &NextCue, window, cx| {
                this.step_cue(true, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &AddCue, _, cx| {
                this.add_cue(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &ToggleLoop, _, cx| {
                this.timeline.looping = !this.timeline.looping;
                cx.notify();
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &ToggleLive, _, cx| {
                this.force_live = !this.force_live;
                cx.notify();
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &TileOutput, window, cx| {
                this.tile(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &PopOut, _, cx| {
                this.release_output(cx);
                cx.stop_propagation();
            }))
            .on_key_down(
                cx.listener(|this, ev: &KeyDownEvent, window, cx| this.key_down(ev, window, cx)),
            )
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| this.key_up(ev, cx)))
            .size_full()
            .bg(background)
            .text_color(foreground)
            .p_4()
            .gap_3()
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .items_start()
                    .child(rail)
                    .child(main),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(match &self.learning {
                        Some(LearnTarget::Trigger(t)) => {
                            format!("press a key to bind trigger `{t}` · esc cancels")
                        }
                        Some(LearnTarget::Cue(id)) => {
                            format!("press a key to bind cue {id} · esc cancels")
                        }
                        None => self.status.clone(),
                    }),
            )
    }
}

/// Longest-path depth per node of a UI structure (sources are 0).
fn structure_depths(graph: &GraphStructure) -> BTreeMap<NodeId, usize> {
    let mut depths: BTreeMap<NodeId, usize> = BTreeMap::new();
    for _ in 0..graph.nodes.len().max(1) {
        let mut changed = false;
        for node in &graph.nodes {
            let depth = node
                .inputs
                .iter()
                .filter_map(|l| l.from.as_ref())
                .filter_map(|from| depths.get(from))
                .map(|d| d + 1)
                .max()
                .unwrap_or(0);
            if depths.get(&node.id) != Some(&depth) {
                depths.insert(node.id.clone(), depth);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    depths
}

/// Two cues so the first run already demonstrates interpolation on the
/// image → warp → feedback graph.
fn default_timeline() -> Timeline {
    let mut timeline = Timeline::new(8.0);
    let values = |amount: f32, twist: f32, decay: f32, zoom: f32, rotate: f32| {
        BTreeMap::from([
            (ParamPath::new("warp", "amount"), ParamValue::Float(amount)),
            (ParamPath::new("warp", "twist"), ParamValue::Float(twist)),
            (
                ParamPath::new("feedback", "decay"),
                ParamValue::Float(decay),
            ),
            (ParamPath::new("feedback", "zoom"), ParamValue::Float(zoom)),
            (
                ParamPath::new("feedback", "rotate"),
                ParamValue::Float(rotate),
            ),
        ])
    };
    timeline.add_cue(0.0, Transition::Cut, values(0.04, 0.0, 0.80, 1.000, 0.0));
    timeline.add_cue(
        4.0,
        Transition::Interpolate,
        values(0.30, 1.2, 0.95, 1.025, 0.012),
    );
    timeline
}
