//! The visible 3D scene: a quad textured with the graph's output.

use bevy::prelude::*;

use crate::nodes::Runtime;
use crate::plugin::NodeResolution;
use crate::{CAMERA_DISTANCE, CAMERA_FOV};

#[derive(Component)]
pub struct DisplayQuad;

pub fn spawn_display(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    runtime: Res<Runtime>,
    resolution: Res<NodeResolution>,
) {
    // Size the quad so it exactly fills the perspective camera's view at
    // CAMERA_DISTANCE for the node aspect ratio; other window aspects letterbox.
    let height = 2.0 * CAMERA_DISTANCE * (CAMERA_FOV * 0.5).tan();
    let width = height * resolution.aspect();
    let mesh = meshes.add(Rectangle::new(width, height));
    let material = materials.add(StandardMaterial {
        base_color_texture: Some(runtime.output_handle()),
        unlit: true,
        ..Default::default()
    });
    commands.spawn((
        Name::new("display quad"),
        DisplayQuad,
        Mesh3d(mesh),
        MeshMaterial3d(material),
        Transform::IDENTITY,
    ));
}

/// Keep the display material pointed at the graph output (the feedback
/// ping-pong means the output handle can change every frame).
pub fn track_output(
    runtime: Res<Runtime>,
    quads: Query<&MeshMaterial3d<StandardMaterial>, With<DisplayQuad>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let handle = runtime.output_handle();
    for quad in &quads {
        if let Some(material) = materials.get(quad.id())
            && material.base_color_texture.as_ref() != Some(&handle)
            && let Some(mut material) = materials.get_mut(quad.id())
        {
            material.base_color_texture = Some(handle.clone());
        }
    }
}
