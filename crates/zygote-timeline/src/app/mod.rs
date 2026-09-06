//! Zygote timeline: state, wiring and the window.
//!
//! `run` opens the window; `TimelineApp` is the whole UI state; the modules
//! below split behaviour by concern. See the note above the `mod` lines.

use std::cell::Cell;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::slider::{Slider, SliderEvent, SliderState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme, Disableable, IconName, Root, Sizable, h_flex, v_flex};
use gpui_kit::prelude::*;
use gpui_kit::*;
use zygote_core::{
    DEFAULT_PORT, GateEvent, GateLog, Graph, GraphStructure, KeyAction, LfoShape, Message,
    ModContext, ModSource, NodeId, NodeLibrary, PROTOCOL_VERSION, ParamDescriptor, ParamPath,
    ParamSender, ParamType, ParamValue, SourceKind, Timeline, Transition,
    WindowBounds as OutputBounds, apply_offset,
};

// One state store, `TimelineApp`, and two kinds of module around it:
//
// * logic modules (`transport`, `controls`, `modulation`, `net`, `keys`,
//   `output`) mutate the store and talk to the renderer; they never build
//   elements. Think custom hooks / reducers.
// * `views` render regions of the window from the store; they never own
//   state. Think function components taking the store as props.
//
// Every module is `impl TimelineApp`, so `self` is the store everywhere and
// events flow back through `cx.listener(|this, ...| ...)`.
mod controls;
mod keys;
mod modulation;
mod net;
mod output;
mod transport;
mod views;

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
    gpui_kit::application()
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx| {
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
    show_help: bool,
    /// Set by a node card's click so the canvas click underneath does not
    /// clear the selection it just made.
    graph_node_clicked: bool,
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
            show_help: false,
            graph_node_clicked: false,
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
