use std::cell::Cell;
use std::collections::BTreeMap;
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
    DEFAULT_PORT, Graph, GraphStructure, Message, NodeId, NodeLibrary, PROTOCOL_VERSION,
    ParamDescriptor, ParamPath, ParamSender, ParamType, ParamValue, Timeline, Transition,
};

/// UI refresh / send rate.
const TICK: Duration = Duration::from_millis(33);
/// How long to wait for the renderer's `Describe` before falling back to the
/// built-in first-pass graph for the parameter list.
const DESCRIBE_TIMEOUT: Duration = Duration::from_millis(1500);
/// Re-send `Hello` this often while the renderer is silent, so a restarted
/// renderer is picked up (and re-fed the current values) automatically.
const HELLO_INTERVAL: Duration = Duration::from_secs(2);
const TIMELINE_HEIGHT: f32 = 110.0;
const NODE_W: f32 = 168.0;
const NODE_H: f32 = 46.0;
const COL_GAP: f32 = 48.0;
const ROW_GAP: f32 = 14.0;
const GRAPH_PAD: f32 = 16.0;
/// Height of the pannable graph viewport.
const GRAPH_VIEW_H: f32 = 150.0;

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
        let bounds = Bounds::centered(None, size(px(1040.), px(720.)), cx);
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

/// One control bound to a parameter. Floats get a slider, vec2/color get one
/// slider per component, bools a switch, choices a button row.
struct ParamControl {
    desc: ParamDescriptor,
    /// Slider per numeric component (1 for float, 2 for vec2, 3 for color rgb).
    sliders: Vec<Entity<SliderState>>,
    _subscriptions: Vec<Subscription>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    Playhead,
    Cue(u32),
}

pub struct TimelineApp {
    file: PathBuf,
    /// Structure of the renderer's graph, drawn read-only above the axis.
    graph: Option<GraphStructure>,
    /// Pan offset of the graph surface inside its viewport, in pixels.
    graph_pan: Point<f32>,
    /// Mouse position and pan at the start of a graph drag.
    graph_drag: Option<(Point<Pixels>, Point<f32>)>,
    graph_viewport: Rc<Cell<Bounds<Pixels>>>,
    /// Set when a new graph arrives so the next render centres it.
    graph_fit_pending: bool,
    timeline: Timeline,
    playhead: f32,
    playing: bool,
    selected: Option<u32>,
    params: Vec<ParamControl>,
    /// Manual values that beat the programmed cues until released.
    overrides: BTreeMap<ParamPath, ParamValue>,
    /// Last values pushed to the renderer, for diffing.
    sent: BTreeMap<ParamPath, ParamValue>,
    sender: Option<ParamSender>,
    described: bool,
    describe_buffer: Vec<ParamDescriptor>,
    started: Instant,
    last_hello: Instant,
    last_heard: Option<Instant>,
    /// Set once the renderer stops answering; cleared (with a re-describe) when it returns.
    quiet: bool,
    protocol_mismatch: Option<u32>,
    last_tick: Instant,
    drag: Drag,
    axis_bounds: Rc<Cell<Bounds<Pixels>>>,
    status: String,
}

impl TimelineApp {
    fn new(options: Options, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let (timeline, status) = match std::fs::read_to_string(&options.file) {
            Ok(json) => match Timeline::from_json(&json) {
                Ok(t) => (t, format!("loaded {}", options.file.display())),
                Err(e) => (
                    default_timeline(),
                    format!("could not parse {}: {e}", options.file.display()),
                ),
            },
            Err(_) => (
                default_timeline(),
                "new timeline (two demo cues)".to_owned(),
            ),
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

        let mut this = Self {
            file: options.file,
            graph: None,
            graph_pan: Point::default(),
            graph_drag: None,
            graph_viewport: Rc::new(Cell::new(Bounds::default())),
            graph_fit_pending: true,
            timeline,
            playhead: 0.0,
            playing: false,
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
            status,
        };
        this.selected = this.timeline.cues.first().map(|c| c.id);

        // Drive the transport, network and slider positions at a fixed rate.
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
            let ranges: Vec<(f32, f32, f32)> = match (&desc.ty, &current) {
                (ParamType::Float { min, max }, v) => {
                    vec![(*min, *max, v.as_float().unwrap_or(*min))]
                }
                (ParamType::Vec2 { min, max }, v) => {
                    let xy = v.as_vec2().unwrap_or([0.0; 2]);
                    vec![(*min, *max, xy[0]), (*min, *max, xy[1])]
                }
                (ParamType::Color, v) => {
                    let c = v.as_color().unwrap_or([1.0; 4]);
                    vec![(0.0, 1.0, c[0]), (0.0, 1.0, c[1]), (0.0, 1.0, c[2])]
                }
                _ => Vec::new(),
            };
            let mut sliders = Vec::new();
            let mut subscriptions = Vec::new();
            for (component, (min, max, value)) in ranges.into_iter().enumerate() {
                let step = step_for(&desc, max - min);
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

    /// A slider moved: manual control of one component of a parameter.
    fn on_component(
        &mut self,
        path: &ParamPath,
        component: usize,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        let current = self.effective_values().get(path).cloned().or_else(|| {
            self.params
                .iter()
                .find(|c| &c.desc.path == path)
                .map(|c| c.desc.value.clone())
        });
        let Some(current) = current else { return };
        let next = match current {
            ParamValue::Float(_) => ParamValue::Float(value),
            ParamValue::Vec2(mut v) => {
                if let Some(slot) = v.get_mut(component) {
                    *slot = value;
                }
                ParamValue::Vec2(v)
            }
            ParamValue::Color(mut c) => {
                if let Some(slot) = c.get_mut(component) {
                    *slot = value;
                }
                ParamValue::Color(c)
            }
            other => other,
        };
        self.set_manual(path, next, cx);
    }

    /// Manual control: this value wins over the cues until released.
    fn set_manual(&mut self, path: &ParamPath, value: ParamValue, cx: &mut Context<Self>) {
        self.overrides.insert(path.clone(), value);
        self.push_values();
        cx.notify();
    }

    fn release_override(&mut self, path: &ParamPath, window: &mut Window, cx: &mut Context<Self>) {
        self.overrides.remove(path);
        if !self.timeline.evaluate(self.playhead).contains_key(path)
            && let Some(sender) = &self.sender
        {
            // Nothing programmed for this parameter: hand it back to the graph's base value.
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
        // Keep datagrams small.
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
        if let Some(sender) = &self.sender
            && self.playing
        {
            let _ = sender.send(&Message::Transport {
                time: self.playhead,
                playing: true,
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
        {
            for msg in messages {
                self.last_heard = Some(Instant::now());
                if self.quiet {
                    // Back from the dead: ask for a fresh description so the
                    // controls match whatever graph it now runs, and re-push state.
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
                                "connected to renderer · graph `{graph}` · {} parameters{}",
                                self.describe_buffer.len(),
                                match self.protocol_mismatch {
                                    Some(p) => format!(
                                        " · protocol mismatch (renderer {p}, ui {PROTOCOL_VERSION})"
                                    ),
                                    None => String::new(),
                                }
                            );
                            let fresh = std::mem::take(&mut self.describe_buffer);
                            // Only rebuild controls when the parameter set actually changed
                            // (a renderer restart re-describes the same graph).
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
        }
        if let Some((descriptors, same)) = installed {
            if !same {
                self.install_params(descriptors, window, cx);
            }
            // Re-send everything so a freshly (re)started renderer picks up the state.
            self.sent.clear();
            self.push_values();
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

        // Heartbeat. A described renderer is pinged; an undescribed or quiet
        // one is re-greeted so a restart is picked up without user action.
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
    }

    fn seek(&mut self, time: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.playhead = time.clamp(0.0, self.timeline.duration);
        self.sync_sliders(window, cx);
        self.push_values();
        cx.notify();
    }

    // ---- cues -------------------------------------------------------------

    fn add_cue(&mut self, cx: &mut Context<Self>) {
        let values = self.effective_values();
        let id = self
            .timeline
            .add_cue(self.playhead, Transition::Interpolate, values);
        self.selected = Some(id);
        self.status = format!("added cue {id} at {:.2}s", self.playhead);
        cx.notify();
    }

    fn update_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let values = self.effective_values();
        if let Some(cue) = self.timeline.cue_mut(id) {
            cue.values = values;
            self.status = format!("updated cue {id}");
        }
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
                self.selected = self.timeline.cues.first().map(|c| c.id);
                self.status = format!("loaded {}", self.file.display());
                self.seek(self.playhead, window, cx);
            }
            Err(e) => self.status = format!("load failed: {e}"),
        }
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
        let t = self.time_at(ev.position.x);
        // Grab a cue if the click lands on its marker, otherwise scrub.
        let width: f32 = self.axis_bounds.get().size.width.into();
        let tolerance = 6.0 / width.max(1.0) * self.timeline.duration;
        let hit = self
            .timeline
            .cues
            .iter()
            .filter(|c| (c.time - t).abs() <= tolerance)
            .min_by(|a, b| (a.time - t).abs().total_cmp(&(b.time - t).abs()))
            .map(|c| c.id);
        match hit {
            Some(id) => {
                self.selected = Some(id);
                self.drag = Drag::Cue(id);
                if let Some(cue) = self.timeline.cue(id) {
                    let time = cue.time;
                    self.seek(time, window, cx);
                }
            }
            None => {
                self.drag = Drag::Playhead;
                self.seek(t, window, cx);
            }
        }
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

    // ---- rendering ---------------------------------------------------------

    fn render_transport(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("play")
                    .small()
                    .primary()
                    .label(if self.playing { "Pause" } else { "Play" })
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
            )
            .child(
                Button::new("stop")
                    .small()
                    .label("Stop")
                    .on_click(cx.listener(|this, _, window, cx| this.stop(window, cx))),
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

    fn render_cue_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.and_then(|id| self.timeline.cue(id));
        let theme = cx.theme();
        h_flex()
            .gap_2()
            .items_center()
            .child(
                Button::new("add-cue")
                    .small()
                    .primary()
                    .label("Add cue at playhead")
                    .on_click(cx.listener(|this, _, _, cx| this.add_cue(cx))),
            )
            .child(
                Button::new("update-cue")
                    .small()
                    .label("Update cue")
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.update_selected(cx))),
            )
            .child(
                Button::new("toggle-transition")
                    .small()
                    .label(match selected.map(|c| c.transition) {
                        Some(Transition::Cut) => "Into cue: cut",
                        Some(Transition::Interpolate) => "Into cue: interpolate",
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
            .child(div().flex_1())
            .child(
                div()
                    .text_sm()
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
            .into_any_element()
    }

    fn render_axis(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let duration = self.timeline.duration.max(1e-3);
        let bounds_cell = self.axis_bounds.clone();
        let accent = theme.primary;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let selected = self.selected;

        let mut axis = div()
            .relative()
            .w_full()
            .h(px(TIMELINE_HEIGHT))
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(theme.muted.opacity(0.35))
            .overflow_hidden()
            .child(
                canvas(move |bounds, _, _| bounds_cell.set(bounds), |_, _, _, _| {})
                    .absolute()
                    .inset_0(),
            );

        // Second ticks with labels. Thin out when the axis gets long.
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
            axis = axis.child(
                div()
                    .absolute()
                    .top_0()
                    .left(relative(frac))
                    .w(px(1.))
                    .h(px(10.))
                    .bg(muted.opacity(0.6)),
            );
            axis = axis.child(
                div()
                    .absolute()
                    .top(px(12.))
                    .left(relative(frac))
                    .ml(px(3.))
                    .text_xs()
                    .text_color(muted)
                    .child(format!("{t:.0}")),
            );
            t += step;
        }

        // Cue markers.
        for cue in &self.timeline.cues {
            let frac = (cue.time / duration).clamp(0.0, 1.0);
            let is_selected = selected == Some(cue.id);
            let color = if is_selected { accent } else { muted };
            axis = axis
                .child(
                    div()
                        .absolute()
                        .top(px(30.))
                        .bottom_0()
                        .left(relative(frac))
                        .w(px(if is_selected { 3. } else { 2. }))
                        .ml(px(if is_selected { -1.5 } else { -1. }))
                        .bg(color),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(30.))
                        .left(relative(frac))
                        .ml(px(-7.))
                        .w(px(14.))
                        .h(px(14.))
                        .rounded_full()
                        .bg(color)
                        .border_1()
                        .border_color(theme.background),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(48.))
                        .left(relative(frac))
                        .ml(px(6.))
                        .text_xs()
                        .text_color(color)
                        .child(format!(
                            "{}{}",
                            cue.label,
                            match cue.transition {
                                Transition::Cut => " ⌐",
                                Transition::Interpolate => " ⟋",
                            }
                        )),
                );
        }

        // Playhead.
        let frac = (self.playhead / duration).clamp(0.0, 1.0);
        axis = axis.child(
            div()
                .absolute()
                .top_0()
                .bottom_0()
                .left(relative(frac))
                .w(px(2.))
                .ml(px(-1.))
                .bg(theme.danger),
        );

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
        // Plain wheel scrolls the chain horizontally; shift (or a trackpad's
        // horizontal axis) moves it the other way.
        if ev.modifiers.shift {
            self.graph_pan = point(self.graph_pan.x + dy, self.graph_pan.y + dx);
        } else {
            self.graph_pan = point(self.graph_pan.x + dx + dy, self.graph_pan.y);
        }
        cx.notify();
    }

    /// Centre the graph in its viewport (left-aligned when it is wider).
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

    /// Read-only picture of the renderer's node graph: sources on the left,
    /// the output on the right, one column per dependency depth.
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

        // Layout: column = depth, row = position within that column, in graph order.
        let depths = structure_depths(&graph);
        let mut rows_in_col: BTreeMap<usize, usize> = BTreeMap::new();
        let mut placed: BTreeMap<zygote_core::NodeId, (usize, usize)> = BTreeMap::new();
        for node in &graph.nodes {
            let col = depths.get(&node.id).copied().unwrap_or(0);
            let row = rows_in_col.entry(col).or_insert(0);
            placed.insert(node.id.clone(), (col, *row));
            *row += 1;
        }
        let cols = rows_in_col.keys().max().map(|c| c + 1).unwrap_or(1);
        let rows = rows_in_col.values().copied().max().unwrap_or(1);
        // One extra column for the window the output feeds.
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

        // Edges are drawn on a canvas underneath the boxes. Collect them as
        // (from right-middle, to left edge at the slot's height).
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
        // Output → window.
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
                            // Arrow head.
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
            let slot_names: Vec<String> = node.inputs.iter().map(|l| l.name.clone()).collect();
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
                    .when(!slot_names.is_empty(), |d| d.child(slots))
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

        // The window the output feeds.
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
                        .child("window"),
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
            let value = values
                .get(&path)
                .cloned()
                .unwrap_or_else(|| control.desc.value.clone());
            let label_color = if overridden {
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
                            this.set_manual(&path, ParamValue::Bool(*checked), cx)
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
                                    this.set_manual(
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
                (ParamType::Float { .. }, _) => match control.sliders.first() {
                    Some(slider) => Slider::new(slider).flex_1().into_any_element(),
                    None => div().into_any_element(),
                },
            };

            list = list.child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(overridden, |d| d.bg(theme.primary.opacity(0.08)))
                    .child(
                        div()
                            .w(px(190.))
                            .pl_3()
                            .text_sm()
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
                        Button::new(("release", i))
                            .xsmall()
                            .label(if overridden { "manual ✕" } else { "cue" })
                            .disabled(!overridden)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.release_override(&path, window, cx)
                            })),
                    ),
            );
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
            .child(div().text_lg().child("Zygote timeline"))
            .child(div().flex_1())
            .child(
                Button::new("release-all")
                    .small()
                    .label("Release all manual overrides")
                    .disabled(self.overrides.is_empty())
                    .on_click(cx.listener(|this, _, window, cx| this.release_all(window, cx))),
            );
        let transport = self.render_transport(cx);
        let graph = self.render_graph(cx);
        let axis = self.render_axis(cx);
        let cue_bar = self.render_cue_bar(cx);
        let params = self.render_params(cx);

        v_flex()
            .size_full()
            .bg(background)
            .text_color(foreground)
            .p_4()
            .gap_3()
            .child(header)
            .child(graph)
            .child(transport)
            .child(axis)
            .child(cue_bar)
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child("Click the axis to scrub, drag a marker to move a cue. Moving a slider takes manual control of that parameter until released."),
            )
            .child(
                div()
                    .id("params")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(params),
            )
            .child(div().text_xs().text_color(muted).child(self.status.clone()))
    }
}

/// Longest-path depth per node of a UI structure (sources are 0).
fn structure_depths(graph: &GraphStructure) -> BTreeMap<NodeId, usize> {
    let mut depths: BTreeMap<NodeId, usize> = BTreeMap::new();
    // Nodes arrive in graph order, which is not necessarily topological; iterate
    // until stable (graphs are small and acyclic).
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

fn step_for(desc: &ParamDescriptor, range: f32) -> f32 {
    let range = range.abs().max(1e-6);
    // Integer-ish float controls snap to whole numbers.
    let integer_like = matches!(
        desc.path.param.as_str(),
        "posterize" | "octaves" | "segments"
    );
    if integer_like { 1.0 } else { range / 1000.0 }
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
