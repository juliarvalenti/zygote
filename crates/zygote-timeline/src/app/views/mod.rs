//! Components. Each file renders one region of the window from `TimelineApp`
//! state; none of them own state of their own.

use super::*;

mod axis;
mod graph;
mod help;
mod params;
mod rail;
mod root;
mod transport;

const AXIS_HEIGHT: f32 = 104.0;
const RAIL_WIDTH: f32 = 200.0;
const NODE_W: f32 = 168.0;
const NODE_H: f32 = 46.0;
const COL_GAP: f32 = 48.0;
const ROW_GAP: f32 = 14.0;
const GRAPH_PAD: f32 = 16.0;
const GRAPH_VIEW_H: f32 = 150.0;
