//! Headless / automated capture: save a screenshot after N frames, then exit.
//! Used for smoke-testing the pipeline without a human at the window.

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

#[derive(Resource, Clone, Debug)]
pub struct CaptureSettings {
    pub path: String,
    /// Frame at which the (last) screenshot is requested.
    pub frame: u64,
    /// When > 0, also save every `every` frames before `frame`, numbering the
    /// files `name-0001.png`, `name-0002.png`, …
    pub every: u64,
}

#[derive(Resource, Default)]
struct CaptureState {
    requested: bool,
    frames: u64,
    saved: u64,
}

pub struct CapturePlugin(pub CaptureSettings);

/// `frames/haze.png`, 7 → `frames/haze-0007.png`
fn numbered(path: &str, index: u64) -> String {
    match path.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !stem.ends_with('/') => {
            format!("{stem}-{index:04}.{ext}")
        }
        _ => format!("{path}-{index:04}.png"),
    }
}

impl Plugin for CapturePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(self.0.clone())
            .init_resource::<CaptureState>()
            .add_systems(Update, capture);
    }
}

fn capture(
    mut commands: Commands,
    settings: Res<CaptureSettings>,
    mut state: ResMut<CaptureState>,
    windows: Query<Entity, With<Window>>,
) {
    state.frames += 1;
    let Some(window) = windows.iter().next() else {
        return;
    };
    if settings.every > 0
        && !state.requested
        && state.frames.is_multiple_of(settings.every)
        && state.frames < settings.frame
    {
        state.saved += 1;
        let path = numbered(&settings.path, state.saved);
        commands
            .spawn(Screenshot::window(window))
            .observe(save_to_disk(path));
    }
    if !state.requested && state.frames >= settings.frame {
        state.requested = true;
        let path = if settings.every > 0 {
            state.saved += 1;
            numbered(&settings.path, state.saved)
        } else {
            settings.path.clone()
        };
        info!("capturing frame {} to {path}", state.frames);
        commands
            .spawn(Screenshot::window(window))
            .observe(save_to_disk(path));
    }
    // Give the GPU readback a generous number of frames to land on disk.
    if state.requested && state.frames >= settings.frame + 60 {
        commands.write_message(AppExit::Success);
    }
}
