use std::cell::Cell;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::{ActiveTheme, Disableable, Root, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use zygote_core::{
    DEFAULT_PORT, Graph, Message, ParamDescriptor, ParamPath, ParamSender, Timeline, Transition,
};

/// UI refresh / send rate.
const TICK: Duration = Duration::from_millis(33);
/// How long to wait for the renderer's `Describe` before falling back to the
/// built-in first-pass graph for the parameter list.
const DESCRIBE_TIMEOUT: Duration = Duration::from_millis(1500);
const TIMELINE_HEIGHT: f32 = 110.0;

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

/// One slider bound to a parameter.
struct ParamControl {
    desc: ParamDescriptor,
    slider: Entity<SliderState>,
    _subscription: Subscription,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Drag {
    None,
    Playhead,
    Cue(u32),
}

pub struct TimelineApp {
    file: PathBuf,
    timeline: Timeline,
    playhead: f32,
    playing: bool,
    selected: Option<u32>,
    params: Vec<ParamControl>,
    /// Manual values that beat the programmed cues until released.
    overrides: BTreeMap<ParamPath, f32>,
    /// Last values pushed to the renderer, for diffing.
    sent: BTreeMap<ParamPath, f32>,
    sender: Option<ParamSender>,
    described: bool,
    describe_buffer: Vec<ParamDescriptor>,
    started: Instant,
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
                let _ = s.send(&Message::Hello {
                    client: "zygote-timeline".to_owned(),
                });
                Some(s)
            }
            Err(e) => {
                eprintln!("cannot open udp socket towards {}: {e}", options.target);
                None
            }
        };

        let mut this = Self {
            file: options.file,
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
            let current = values.get(&desc.path).copied().unwrap_or(desc.value);
            let slider = cx.new(|_| {
                SliderState::new()
                    .min(desc.min)
                    .max(desc.max)
                    .step(step_for(&desc))
                    .default_value(current)
            });
            let path = desc.path.clone();
            let subscription = cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
                if let SliderEvent::Change(value) = event {
                    this.on_slider(&path, value.start(), cx);
                }
            });
            self.params.push(ParamControl {
                desc,
                slider,
                _subscription: subscription,
            });
        }
        self.sync_sliders(window, cx);
        cx.notify();
    }

    fn on_slider(&mut self, path: &ParamPath, value: f32, cx: &mut Context<Self>) {
        // Manual control: this value wins over the cues until released.
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
    fn effective_values(&self) -> BTreeMap<ParamPath, f32> {
        let mut values = self.timeline.evaluate(self.playhead);
        for (path, value) in &self.overrides {
            values.insert(path.clone(), *value);
        }
        values
    }

    /// Send whatever changed since the last push.
    fn push_values(&mut self) {
        let Some(sender) = &self.sender else { return };
        let values = self.effective_values();
        let changed: Vec<(ParamPath, f32)> = values
            .iter()
            .filter(|(path, value)| self.sent.get(*path) != Some(*value))
            .map(|(path, value)| (path.clone(), *value))
            .collect();
        if changed.is_empty() {
            return;
        }
        // Keep datagrams small.
        for chunk in changed.chunks(48) {
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
            let Some(value) = values.get(&control.desc.path).copied() else {
                continue;
            };
            control.slider.update(cx, |slider, cx| {
                if (slider.value().start() - value).abs() > 1e-6 {
                    slider.set_value(value, window, cx);
                }
            });
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
        if let Some(sender) = &mut self.sender {
            for msg in sender.poll() {
                if let Message::Describe {
                    graph,
                    chunk,
                    chunks,
                    params,
                } = msg
                {
                    if chunk == 0 {
                        self.describe_buffer.clear();
                    }
                    self.describe_buffer.extend(params);
                    if chunk + 1 == chunks {
                        self.described = true;
                        self.status = format!(
                            "connected to renderer · graph `{graph}` · {} parameters",
                            self.describe_buffer.len()
                        );
                        installed = Some(std::mem::take(&mut self.describe_buffer));
                    }
                }
            }
        }
        if let Some(descriptors) = installed {
            self.install_params(descriptors, window, cx);
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
            self.install_params(Graph::first_pass().describe_params(), window, cx);
            self.described = true;
        }
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
        for (i, control) in self.params.iter().enumerate() {
            let path = control.desc.path.clone();
            let overridden = self.overrides.contains_key(&path);
            let value = values.get(&path).copied().unwrap_or(control.desc.value);
            let label_color = if overridden {
                theme.primary
            } else {
                theme.foreground
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
                            .text_sm()
                            .text_color(label_color)
                            .child(control.desc.label.clone()),
                    )
                    .child(Slider::new(&control.slider).flex_1())
                    .child(
                        div()
                            .w(px(64.))
                            .text_sm()
                            .font_family("monospace")
                            .text_color(theme.muted_foreground)
                            .child(format!("{value:.3}")),
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

fn step_for(desc: &ParamDescriptor) -> f32 {
    let range = (desc.max - desc.min).abs().max(1e-6);
    // Integer-ish controls (blend mode, palette, posterize, octaves) snap to whole numbers.
    let integer_like = matches!(
        desc.path.param.as_str(),
        "mode" | "palette" | "posterize" | "octaves"
    );
    if integer_like { 1.0 } else { range / 1000.0 }
}

/// Two cues so the first run already demonstrates interpolation on the
/// image → warp → feedback graph.
fn default_timeline() -> Timeline {
    let mut timeline = Timeline::new(8.0);
    let values = |amount: f32, twist: f32, decay: f32, zoom: f32, rotate: f32| {
        BTreeMap::from([
            (ParamPath::new("warp", "amount"), amount),
            (ParamPath::new("warp", "twist"), twist),
            (ParamPath::new("feedback", "decay"), decay),
            (ParamPath::new("feedback", "zoom"), zoom),
            (ParamPath::new("feedback", "rotate"), rotate),
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
