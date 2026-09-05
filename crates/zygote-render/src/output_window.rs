//! The output window and its perspective camera, plus the keep-alive that
//! re-creates the window if the OS takes it away (monitor unplugged, dock
//! sleep). A close request from the user is a quit, not a loss.
//!
//! Navigation in the output window: wheel dollies in and out, left-drag pans,
//! right-drag orbits around the display quad, arrow keys pan, `+`/`-` dolly,
//! `Home` or `0` resets. `S` saves a screenshot, `Esc` quits.

use bevy::camera::visibility::RenderLayers;
use bevy::camera::{Hdr, PerspectiveProjection, Projection, RenderTarget};
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};
use bevy::window::{PrimaryWindow, WindowCloseRequested, WindowRef};

use crate::{CAMERA_DISTANCE, CAMERA_FOV};

pub struct OutputWindowPlugin;

impl Plugin for OutputWindowPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<View>()
            .init_resource::<Quitting>()
            .add_systems(Startup, spawn_output)
            .add_systems(
                Update,
                (
                    close_requested,
                    keep_window_alive,
                    keys,
                    navigate,
                    apply_view,
                )
                    .chain(),
            );
    }
}

#[derive(Component)]
pub struct OutputCamera;

/// Set once the user asked to close the window or quit; the keep-alive must
/// not fight that.
#[derive(Resource, Default)]
struct Quitting(bool);

/// The close button means quit, exactly like `Esc`. Bevy's default handler
/// despawns the window; without this the keep-alive would recreate it.
fn close_requested(
    mut requests: MessageReader<WindowCloseRequested>,
    mut quitting: ResMut<Quitting>,
    mut exit: MessageWriter<AppExit>,
) {
    if requests.read().next().is_some() && !quitting.0 {
        info!("window close requested; quitting");
        quitting.0 = true;
        exit.write(AppExit::Success);
    }
}

/// Orbit-camera state around the display quad.
#[derive(Resource, Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Point the camera looks at (the quad is at the origin).
    pub target: Vec3,
    /// Distance from the target.
    pub distance: f32,
    /// Orbit angles in radians: yaw around Y, pitch around the camera's right axis.
    pub yaw: f32,
    pub pitch: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            target: Vec3::ZERO,
            distance: CAMERA_DISTANCE,
            yaw: 0.0,
            pitch: 0.0,
        }
    }
}

impl View {
    const MIN_DISTANCE: f32 = 0.15;
    const MAX_DISTANCE: f32 = 12.0;
    const MAX_PITCH: f32 = 1.45;

    fn rotation(&self) -> Quat {
        Quat::from_rotation_y(self.yaw) * Quat::from_rotation_x(-self.pitch)
    }

    fn transform(&self) -> Transform {
        let rotation = self.rotation();
        let position = self.target + rotation * Vec3::new(0.0, 0.0, self.distance);
        Transform::from_translation(position).with_rotation(rotation)
    }

    /// Move closer or further by a factor (positive = in).
    fn dolly(&mut self, steps: f32) {
        self.distance =
            (self.distance * (0.9_f32).powf(steps)).clamp(Self::MIN_DISTANCE, Self::MAX_DISTANCE);
    }

    /// Pan in the camera's plane by a fraction of the visible height.
    fn pan(&mut self, dx: f32, dy: f32) {
        let rotation = self.rotation();
        let visible_height = 2.0 * self.distance * (CAMERA_FOV * 0.5).tan();
        let right = rotation * Vec3::X;
        let up = rotation * Vec3::Y;
        self.target -= right * dx * visible_height;
        self.target += up * dy * visible_height;
        self.target = self.target.clamp(Vec3::splat(-6.0), Vec3::splat(6.0));
    }

    fn orbit(&mut self, dyaw: f32, dpitch: f32) {
        self.yaw = (self.yaw + dyaw).rem_euclid(std::f32::consts::TAU);
        self.pitch = (self.pitch + dpitch).clamp(-Self::MAX_PITCH, Self::MAX_PITCH);
    }
}

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
    quitting: Res<Quitting>,
    mut missing_for: Local<u32>,
) {
    if quitting.0 || windows.iter().next().is_some() {
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

/// Mouse and keyboard navigation of the output view.
fn navigate(
    mut view: ResMut<View>,
    keys: Res<ButtonInput<KeyCode>>,
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    windows: Query<&Window, With<PrimaryWindow>>,
    time: Res<Time<Real>>,
) {
    let Some(window) = windows.iter().next() else {
        return;
    };
    let height = window.height().max(1.0);
    let mut next = *view;

    // Wheel: dolly. One notch (line) is one step; pixel deltas are scaled.
    let wheel = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if wheel != 0.0 {
        next.dolly(wheel);
    }
    // Drag: left pans, right (or middle) orbits.
    if motion.delta != Vec2::ZERO {
        if buttons.pressed(MouseButton::Left) {
            next.pan(motion.delta.x / height, motion.delta.y / height);
        } else if buttons.pressed(MouseButton::Right) || buttons.pressed(MouseButton::Middle) {
            next.orbit(-motion.delta.x * 0.005, motion.delta.y * 0.005);
        }
    }
    // Keys: arrows pan, +/- dolly, Home/0 reset.
    let dt = time.delta_secs();
    let mut pan = Vec2::ZERO;
    if keys.pressed(KeyCode::ArrowLeft) {
        pan.x += 1.0;
    }
    if keys.pressed(KeyCode::ArrowRight) {
        pan.x -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowUp) {
        pan.y -= 1.0;
    }
    if keys.pressed(KeyCode::ArrowDown) {
        pan.y += 1.0;
    }
    if pan != Vec2::ZERO {
        next.pan(pan.x * dt * 0.6, pan.y * dt * 0.6);
    }
    if keys.pressed(KeyCode::Equal) || keys.pressed(KeyCode::NumpadAdd) {
        next.dolly(dt * 6.0);
    }
    if keys.pressed(KeyCode::Minus) || keys.pressed(KeyCode::NumpadSubtract) {
        next.dolly(-dt * 6.0);
    }
    if keys.just_pressed(KeyCode::Home)
        || keys.just_pressed(KeyCode::Digit0)
        || keys.just_pressed(KeyCode::Numpad0)
    {
        next = View::default();
    }
    if next != *view {
        *view = next;
    }
}

fn apply_view(view: Res<View>, mut cameras: Query<&mut Transform, With<OutputCamera>>) {
    if !view.is_changed() {
        return;
    }
    for mut transform in &mut cameras {
        *transform = view.transform();
    }
}

fn keys(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<Entity, With<PrimaryWindow>>,
    mut quitting: ResMut<Quitting>,
    mut exit: MessageWriter<AppExit>,
) {
    if keys.just_pressed(KeyCode::Escape) {
        quitting.0 = true;
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
