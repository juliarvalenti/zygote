//! The output window and its perspective camera, plus the keep-alive that
//! re-creates the window if the OS takes it away (monitor unplugged, dock
//! sleep). Keys: `S` saves a screenshot, `Esc` quits.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Hdr, PerspectiveProjection, Projection, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PrimaryWindow, WindowRef};

use crate::{CAMERA_DISTANCE, CAMERA_FOV};

pub struct OutputWindowPlugin;

impl Plugin for OutputWindowPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_output)
            .add_systems(Update, (keep_window_alive, keys));
    }
}

#[derive(Component)]
pub struct OutputCamera;

fn output_window() -> Window {
    Window {
        title: "Zygote".to_owned(),
        resolution: (1280, 720).into(),
        ..Default::default()
    }
}

fn spawn_output(mut commands: Commands) {
    commands.spawn((Name::new("output window"), output_window(), PrimaryWindow));
    // 3D from the start: a perspective camera with a depth buffer looking down
    // -Z at the display quad. Layer 0 is the window layer.
    commands.spawn((
        Name::new("output camera"),
        OutputCamera,
        Camera3d::default(),
        Camera {
            clear_color: ClearColorConfig::Custom(Color::BLACK),
            ..Default::default()
        },
        Hdr,
        Tonemapping::None,
        Projection::Perspective(PerspectiveProjection {
            fov: CAMERA_FOV,
            near: 0.05,
            far: 100.0,
            ..Default::default()
        }),
        Transform::from_xyz(0.0, 0.0, CAMERA_DISTANCE).looking_at(Vec3::ZERO, Vec3::Y),
        RenderTarget::Window(WindowRef::Primary),
        RenderLayers::layer(0),
    ));
}

/// If the primary window vanished (monitor removed, closed by the OS), make a
/// new one on whatever display remains. The camera targets `WindowRef::Primary`
/// so it follows automatically.
fn keep_window_alive(
    mut commands: Commands,
    windows: Query<Entity, With<Window>>,
    mut missing_for: Local<u32>,
) {
    if windows.iter().next().is_some() {
        *missing_for = 0;
        return;
    }
    *missing_for += 1;
    // A couple of frames of grace so a window that is mid-recreation by the
    // platform does not get doubled.
    if *missing_for == 3 {
        warn!("output window disappeared; re-creating it on the remaining display");
        commands.spawn((Name::new("output window"), output_window(), PrimaryWindow));
    }
}

fn keys(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        exit.write(AppExit::Success);
    }
    if keys.just_pressed(KeyCode::KeyS)
        && let Some(window) = windows.iter().next()
    {
        let path = format!(
            "zygote-{}.png",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        );
        info!("saving screenshot to {path}");
        commands
            .spawn(Screenshot::window(window))
            .observe(save_to_disk(path));
    }
}
