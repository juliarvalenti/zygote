//! Zygote render process.
//!
//! nannou (Bevy) window with a perspective camera looking at a quad that shows
//! the output of a reconfigurable node graph. Parameters arrive over UDP from
//! the timeline UI (`zygote-timeline`) or any other client speaking
//! [`zygote_core::Message`].
//!
//! ```text
//! zygote-render [graph.json] [--port N] [--showcase] [--size WxH] [--assets DIR]
//!               [--capture out.png [--frames N]]
//! ```
//!
//! Keys: `S` saves a screenshot, `Esc` quits.

mod capture;
mod materials;
mod net;
mod nodes;
mod params;
mod plugin;
mod scene;

use bevy::camera::{Hdr, PerspectiveProjection, Projection};
use bevy::core_pipeline::tonemapping::Tonemapping;
use nannou::prelude::*;
use zygote_core::Graph;

use crate::plugin::{RenderSettings, ZygotePlugin};

/// Distance from the perspective camera to the display quad.
pub const CAMERA_DISTANCE: f32 = 3.0;
/// Vertical field of view of the display camera.
pub const CAMERA_FOV: f32 = 45.0_f32.to_radians();

struct Model {
    _window: Entity,
}

fn main() {
    let (settings, capture) = parse_args();
    let mut builder = nannou::app(model)
        .update(update)
        .add_plugin(ZygotePlugin::new(settings));
    if let Some(capture) = capture {
        builder = builder.add_plugin(capture::CapturePlugin(capture));
    }
    builder.run();
}

/// Bevy resolves `assets/` relative to `BEVY_ASSET_ROOT`, then the cargo
/// manifest dir, then the executable. Outside `cargo run` neither of the last
/// two points at this crate, so pick a sensible root before the app starts.
fn resolve_asset_root(explicit: Option<String>) {
    if std::env::var_os("BEVY_ASSET_ROOT").is_some() && explicit.is_none() {
        return;
    }
    let candidates = [
        explicit,
        Some(env!("CARGO_MANIFEST_DIR").to_owned()),
        std::env::current_dir()
            .ok()
            .map(|d| d.display().to_string()),
        std::env::current_dir()
            .ok()
            .map(|d| d.join("crates/zygote-render").display().to_string()),
    ];
    for root in candidates.into_iter().flatten() {
        if std::path::Path::new(&root).join("assets").is_dir() {
            // SAFETY: called from `main` before any other thread exists.
            unsafe { std::env::set_var("BEVY_ASSET_ROOT", &root) };
            return;
        }
    }
}

fn parse_args() -> (RenderSettings, Option<capture::CaptureSettings>) {
    let mut settings = RenderSettings::default();
    let mut assets: Option<String> = None;
    let mut capture_path: Option<String> = None;
    let mut capture_frame: u64 = 120;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--assets" => {
                assets = Some(
                    args.next()
                        .unwrap_or_else(|| usage("--assets needs a directory")),
                );
            }
            "--capture" => {
                capture_path = Some(
                    args.next()
                        .unwrap_or_else(|| usage("--capture needs a path")),
                );
            }
            "--frames" => {
                capture_frame = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or_else(|| usage("--frames needs a number"));
            }
            "--port" => {
                settings.port = args
                    .next()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or_else(|| usage("--port needs a number"));
            }
            "--size" => {
                let value = args.next().unwrap_or_else(|| usage("--size needs WxH"));
                let (w, h) = value
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse().ok()?, h.parse().ok()?)))
                    .unwrap_or_else(|| usage("--size needs WxH"));
                settings.resolution = UVec2::new(w, h);
            }
            "--showcase" => settings.graph = Graph::showcase(),
            "-h" | "--help" => usage(""),
            path if !path.starts_with('-') => {
                let json = std::fs::read_to_string(path)
                    .unwrap_or_else(|e| usage(&format!("cannot read {path}: {e}")));
                settings.graph = Graph::from_json(&json)
                    .unwrap_or_else(|e| usage(&format!("invalid graph {path}: {e}")));
            }
            other => usage(&format!("unknown argument {other}")),
        }
    }
    if let Err(e) = settings.graph.validate() {
        usage(&format!("graph is invalid: {e}"));
    }
    resolve_asset_root(assets);
    let capture = capture_path.map(|path| capture::CaptureSettings {
        path,
        frame: capture_frame,
    });
    (settings, capture)
}

fn usage(error: &str) -> ! {
    if !error.is_empty() {
        eprintln!("error: {error}\n");
    }
    eprintln!(
        "usage: zygote-render [graph.json] [--port N] [--showcase] [--size WxH] [--assets DIR] [--capture out.png [--frames N]]"
    );
    std::process::exit(if error.is_empty() { 0 } else { 2 });
}

fn model(app: &App) -> Model {
    // 3D from the start: a perspective camera with a depth buffer, looking down
    // -Z at the display quad spawned by `scene::spawn_display`.
    let camera = app
        .new_camera()
        .projection(Projection::Perspective(PerspectiveProjection {
            fov: CAMERA_FOV,
            near: 0.05,
            far: 100.0,
            ..Default::default()
        }))
        .xyz(Vec3::new(0.0, 0.0, CAMERA_DISTANCE))
        .tonemapping(Tonemapping::None)
        .clear_color(ClearColorConfig::Custom(Color::BLACK))
        .build();
    app.command_scope(|mut commands| {
        // Keep the main pass in 16-bit float so gradients from the node chain
        // are not re-quantised before hitting the swapchain.
        commands.entity(camera).insert(Hdr);
    });

    let window = app
        .new_window()
        .window(bevy::window::Window {
            title: "Zygote".to_owned(),
            ..Default::default()
        })
        .camera(camera)
        .primary()
        .key_pressed(key_pressed)
        .build();

    Model { _window: window }
}

fn update(_app: &App, _model: &mut Model) {}

fn key_pressed(app: &App, _model: &mut Model, key: KeyCode) {
    match key {
        KeyCode::KeyS => {
            let path = format!(
                "zygote-{}.png",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            );
            info!("saving screenshot to {path}");
            app.main_window().save_screenshot(path);
        }
        KeyCode::Escape => app.quit(),
        _ => {}
    }
}
