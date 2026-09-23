//! A webcam seen through a thermal imager. One instrument, several modes:
//! the `camera` node is pixelated to a coarse sensor grid, false-colored by
//! luminance, traced with contour edges and given a heat bloom. The cues are
//! the imager's palette modes (ironbow, contour, arctic, rainbow, white hot,
//! heat rise), so every look reads as the same device.
//!
//! Keys: `1`–`6` jump to a mode, `q` flares the bloom, `w` drops the sensor
//! resolution like a recalibration click. The camera stays live while the
//! transport is paused. `ZYGOTE_CAMERA=synthetic` runs it without a camera.

use zygote_render::prelude::*;

fn main() {
    ZygoteApp::new()
        .asset_root(env!("CARGO_MANIFEST_DIR"))
        .graph_file("graphs/main.json")
        .parse_args()
        .run();
}
