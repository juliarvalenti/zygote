//! Headless / automated capture: save a screenshot after N frames, then exit.
//! Used for smoke-testing the pipeline without a human at the window.

use bevy::app::AppExit;
use bevy::prelude::*;
use bevy::render::view::screenshot::{Screenshot, save_to_disk};

#[derive(Resource, Clone, Debug)]
pub struct CaptureSettings {
    pub path: String,
    /// Frame at which the screenshot is requested.
    pub frame: u64,
}

#[derive(Resource, Default)]
struct CaptureState {
    requested: bool,
    frames: u64,
}

pub struct CapturePlugin(pub CaptureSettings);

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
    if !state.requested && state.frames >= settings.frame {
        let Some(window) = windows.iter().next() else {
            return;
        };
        state.requested = true;
        info!("capturing frame {} to {}", state.frames, settings.path);
        commands
            .spawn(Screenshot::window(window))
            .observe(save_to_disk(settings.path.clone()));
    }
    // Give the GPU readback a generous number of frames to land on disk.
    if state.requested && state.frames >= settings.frame + 60 {
        commands.write_message(AppExit::Success);
    }
}
