//! Live camera sources.
//!
//! A *device* is a string in the graph (`"0"`, a name fragment,
//! `"synthetic"`, later `"ilidar://host:port"`). [`Cameras`] opens each
//! device once and every `camera` node naming it reads the same frames, so
//! two nodes can show colour and depth of one feed. A backend is a capture
//! thread that fills a [`Slot`] with the newest [`Frame`]; the render side
//! never waits on it. Frames carry a colour plane and an optional depth
//! plane from day one; the webcam backend leaves depth empty.
//!
//! Each frame the newest capture is written into the node's texture,
//! honouring the node's `mirror`, `output` and `on_pause` parameters. With
//! `on_pause = hold` a paused transport freezes the camera too, so the
//! whole picture freezes together; the feed cannot be scrubbed backwards,
//! like any CPU source.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bevy::asset::RenderAssetUsages;
use bevy::image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use zygote_core::{NodeId, ParamPath, ParamValue};

use crate::params::{FrameClock, ParamState};

/// Environment variable that overrides every camera node's device; lets a
/// machine without a camera (CI) run camera graphs on the synthetic feed.
pub const DEVICE_OVERRIDE: &str = "ZYGOTE_CAMERA";

/// One captured frame: tightly packed RGBA8 plus an optional depth plane.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub depth: Option<DepthPlane>,
}

/// Depth in device units (millimetres for the sensors we expect); 0 means
/// unknown. May be a different size from the colour plane.
pub struct DepthPlane {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u16>,
    /// Value mapped to full brightness / alpha when the plane is displayed.
    pub range: u16,
}

/// What a capture thread fills and the renderer drains.
#[derive(Default)]
pub struct Slot {
    pub frame: Option<Frame>,
    pub error: Option<String>,
}

pub type SharedSlot = Arc<Mutex<Slot>>;

/// A device that has been opened: its slot and the newest frame taken so far.
struct Opened {
    slot: SharedSlot,
    latest: Option<Arc<Frame>>,
    version: u64,
    reported: bool,
}

#[derive(Resource, Default)]
pub struct Cameras {
    devices: HashMap<String, Opened>,
}

impl Cameras {
    /// Resolve the device name (honouring [`DEVICE_OVERRIDE`]) and start
    /// capturing if this is the first node to ask for it. Returns the name
    /// nodes should read from.
    pub fn open(&mut self, device: &str) -> String {
        let device = match std::env::var(DEVICE_OVERRIDE) {
            Ok(over) if !over.is_empty() => {
                if over != device {
                    info!("camera `{device}` overridden to `{over}` by {DEVICE_OVERRIDE}");
                }
                over
            }
            _ => device.to_owned(),
        };
        if !self.devices.contains_key(&device) {
            let slot: SharedSlot = Arc::default();
            backends::start(&device, slot.clone());
            self.devices.insert(
                device.clone(),
                Opened {
                    slot,
                    latest: None,
                    version: 0,
                    reported: false,
                },
            );
        }
        device
    }

    fn latest(&self, device: &str) -> Option<(u64, Arc<Frame>)> {
        let d = self.devices.get(device)?;
        d.latest.clone().map(|f| (d.version, f))
    }
}

/// A `camera` node's texture and what it last showed.
pub struct CameraNode {
    node: NodeId,
    device: String,
    image: Handle<Image>,
    shown: u64,
}

impl CameraNode {
    pub fn new(node: NodeId, device: String, image: Handle<Image>) -> Self {
        Self {
            node,
            device,
            image,
            shown: 0,
        }
    }
}

#[derive(Resource, Default)]
pub struct CameraNodes(pub Vec<CameraNode>);

/// Black 2×2 until the first frame arrives.
pub fn placeholder_image() -> Image {
    let mut image = Image::new(
        Extent3d {
            width: 2,
            height: 2,
            ..Default::default()
        },
        TextureDimension::D2,
        [0, 0, 0, 255].repeat(4),
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
    );
    image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
        address_mode_u: ImageAddressMode::ClampToEdge,
        address_mode_v: ImageAddressMode::ClampToEdge,
        address_mode_w: ImageAddressMode::ClampToEdge,
        mag_filter: ImageFilterMode::Linear,
        min_filter: ImageFilterMode::Linear,
        mipmap_filter: ImageFilterMode::Linear,
        ..Default::default()
    });
    image
}

/// Drain every device's slot: newest frame wins, errors are logged once.
pub fn poll_cameras(mut cameras: ResMut<Cameras>) {
    for (name, opened) in cameras.devices.iter_mut() {
        let Ok(mut slot) = opened.slot.lock() else {
            continue;
        };
        if let Some(frame) = slot.frame.take() {
            opened.latest = Some(Arc::new(frame));
            opened.version += 1;
        }
        if let Some(err) = slot.error.take()
            && !opened.reported
        {
            error!("camera `{name}`: {err}");
            opened.reported = true;
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Output {
    Colour,
    Depth,
    ColourDepthAlpha,
}

/// Write the newest frame into each camera node's texture.
pub fn upload_cameras(
    cameras: Res<Cameras>,
    mut nodes: ResMut<CameraNodes>,
    state: Res<ParamState>,
    clock: Res<FrameClock>,
    mut images: ResMut<Assets<Image>>,
) {
    for node in nodes.0.iter_mut() {
        let Some((version, frame)) = cameras.latest(&node.device) else {
            continue;
        };
        if version == node.shown {
            continue;
        }
        let param = |name: &str| {
            state
                .resolved
                .get(&ParamPath::new(node.node.clone(), name.to_owned()))
                .cloned()
        };
        let hold = matches!(param("on_pause"), Some(ParamValue::Choice(c)) if c == "hold");
        // The first frame always shows; after that a paused transport holds.
        if hold && clock.dt <= 0.0 && node.shown != 0 {
            continue;
        }
        let mirror = matches!(param("mirror"), Some(ParamValue::Bool(true)));
        let output = match param("output") {
            Some(ParamValue::Choice(c)) if c == "depth" => Output::Depth,
            Some(ParamValue::Choice(c)) if c == "colour_depth_alpha" => Output::ColourDepthAlpha,
            _ => Output::Colour,
        };
        let Some(mut image) = images.get_mut(&node.image) else {
            continue;
        };
        let (width, height, pixels) = compose(&frame, output, mirror);
        if image.width() != width || image.height() != height {
            image.resize(Extent3d {
                width,
                height,
                ..Default::default()
            });
        }
        image.data = Some(pixels);
        node.shown = version;
    }
}

/// Depth at colour-plane pixel (x, y), scaled to 0..255, or 255 (near) when
/// the frame has no depth so `colour_depth_alpha` degrades to plain colour.
fn depth8(frame: &Frame, x: u32, y: u32) -> u8 {
    let Some(d) = &frame.depth else { return 255 };
    let dx = (x as u64 * d.width as u64 / frame.width.max(1) as u64) as u32;
    let dy = (y as u64 * d.height as u64 / frame.height.max(1) as u64) as u32;
    let v = d.data[(dy * d.width + dx) as usize];
    if v == 0 {
        return 0;
    }
    ((v.min(d.range) as u32 * 255) / d.range.max(1) as u32) as u8
}

fn compose(frame: &Frame, output: Output, mirror: bool) -> (u32, u32, Vec<u8>) {
    let (w, h) = (frame.width, frame.height);
    let mut out = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let sx = if mirror { w - 1 - x } else { x };
            let src = ((y * w + sx) * 4) as usize;
            let dst = ((y * w + x) * 4) as usize;
            match output {
                Output::Colour => {
                    out[dst..dst + 3].copy_from_slice(&frame.rgba[src..src + 3]);
                    out[dst + 3] = 255;
                }
                Output::Depth => {
                    let d = depth8(frame, sx, y);
                    out[dst] = d;
                    out[dst + 1] = d;
                    out[dst + 2] = d;
                    out[dst + 3] = 255;
                }
                Output::ColourDepthAlpha => {
                    out[dst..dst + 3].copy_from_slice(&frame.rgba[src..src + 3]);
                    out[dst + 3] = depth8(frame, sx, y);
                }
            }
        }
    }
    (w, h, out)
}

/// Capture backends. Each one owns a thread and fills the slot; it must
/// never block the caller.
mod backends {
    use super::*;

    pub fn start(device: &str, slot: SharedSlot) {
        if device == "synthetic" || device == "test" || device.starts_with("synthetic:") {
            synthetic::start(device, slot);
            return;
        }
        #[cfg(feature = "camera")]
        {
            nokhwa_backend::start(device, slot);
        }
        #[cfg(not(feature = "camera"))]
        {
            if let Ok(mut s) = slot.lock() {
                s.error = Some(format!(
                    "device `{device}` requested but this build has no camera backend (feature `camera`); use `synthetic`"
                ));
            }
        }
    }

    /// A moving test picture at 30 fps with a depth plane, for machines
    /// without a camera and for exercising the depth outputs.
    pub mod synthetic {
        use super::*;
        use std::time::{Duration, Instant};

        pub fn start(device: &str, slot: SharedSlot) {
            let (w, h) = device
                .strip_prefix("synthetic:")
                .and_then(|s| s.split_once('x'))
                .and_then(|(a, b)| Some((a.parse().ok()?, b.parse().ok()?)))
                .unwrap_or((640u32, 360u32));
            std::thread::Builder::new()
                .name(format!("camera {device}"))
                .spawn(move || {
                    let start = Instant::now();
                    loop {
                        let t = start.elapsed().as_secs_f32();
                        let frame = render(w, h, t);
                        if let Ok(mut s) = slot.lock() {
                            s.frame = Some(frame);
                        }
                        std::thread::sleep(Duration::from_millis(33));
                    }
                })
                .expect("spawn synthetic camera thread");
        }

        fn render(w: u32, h: u32, t: f32) -> Frame {
            let mut rgba = vec![0u8; (w * h * 4) as usize];
            let mut depth = vec![0u16; (w * h) as usize];
            let (cx, cy) = (0.5 + 0.3 * (t * 0.7).cos(), 0.5 + 0.25 * (t * 0.9).sin());
            for y in 0..h {
                for x in 0..w {
                    let (u, v) = (x as f32 / w as f32, y as f32 / h as f32);
                    // A gradient background with a drifting bright disc and a slow bar.
                    let bar = ((u * 6.0 + t * 0.5).fract() < 0.5) as u8 as f32;
                    let dist = ((u - cx) * (w as f32 / h as f32)).hypot(v - cy);
                    let disc = (1.0 - (dist / 0.18).min(1.0)).powf(2.0);
                    let r = 0.15 + 0.5 * u + 0.35 * disc;
                    let g = 0.15 + 0.5 * v * bar + 0.6 * disc;
                    let b = 0.3 + 0.4 * (1.0 - u) + 0.2 * disc;
                    let i = ((y * w + x) * 4) as usize;
                    rgba[i] = (r.min(1.0) * 255.0) as u8;
                    rgba[i + 1] = (g.min(1.0) * 255.0) as u8;
                    rgba[i + 2] = (b.min(1.0) * 255.0) as u8;
                    rgba[i + 3] = 255;
                    // Depth: the disc is near (500 mm), the background recedes to 3 m.
                    let far = 3000.0 - 1500.0 * v;
                    depth[(y * w + x) as usize] = (far - (far - 500.0) * disc) as u16;
                }
            }
            Frame {
                width: w,
                height: h,
                rgba,
                depth: Some(DepthPlane {
                    width: w,
                    height: h,
                    data: depth,
                    range: 3000,
                }),
            }
        }
    }

    /// Real devices through nokhwa. The device string is an index (`"0"`) or
    /// a case-insensitive fragment of the device name.
    #[cfg(feature = "camera")]
    pub mod nokhwa_backend {
        use super::*;
        use nokhwa::pixel_format::RgbAFormat;
        use nokhwa::utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType};
        use nokhwa::{Camera, nokhwa_initialize, query};

        pub fn start(device: &str, slot: SharedSlot) {
            let device = device.to_owned();
            std::thread::Builder::new()
                .name(format!("camera {device}"))
                .spawn(move || {
                    // macOS asks the user for camera access here; other
                    // platforms report `true` immediately.
                    let (tx, rx) = std::sync::mpsc::channel();
                    nokhwa_initialize(move |granted| {
                        let _ = tx.send(granted);
                    });
                    if let Ok(false) = rx.recv() {
                        fail(&slot, "camera access denied by the operating system");
                        return;
                    }
                    let index = match resolve(&device) {
                        Ok(i) => i,
                        Err(e) => return fail(&slot, &e),
                    };
                    let requested = RequestedFormat::new::<RgbAFormat>(
                        RequestedFormatType::AbsoluteHighestFrameRate,
                    );
                    let mut camera = match Camera::new(index, requested) {
                        Ok(c) => c,
                        Err(e) => return fail(&slot, &format!("cannot open `{device}`: {e}")),
                    };
                    if let Err(e) = camera.open_stream() {
                        return fail(&slot, &format!("cannot start `{device}`: {e}"));
                    }
                    info!(
                        "camera `{device}`: {} at {} fps",
                        camera.resolution(),
                        camera.frame_rate()
                    );
                    loop {
                        let buffer = match camera.frame() {
                            Ok(b) => b,
                            Err(e) => return fail(&slot, &format!("`{device}` stopped: {e}")),
                        };
                        let decoded = match buffer.decode_image::<RgbAFormat>() {
                            Ok(d) => d,
                            Err(e) => return fail(&slot, &format!("`{device}` decode: {e}")),
                        };
                        let (width, height) = (decoded.width(), decoded.height());
                        let frame = Frame {
                            width,
                            height,
                            rgba: decoded.into_raw(),
                            depth: None,
                        };
                        if let Ok(mut s) = slot.lock() {
                            s.frame = Some(frame);
                        }
                    }
                })
                .expect("spawn camera thread");
        }

        fn fail(slot: &SharedSlot, msg: &str) {
            if let Ok(mut s) = slot.lock() {
                s.error = Some(msg.to_owned());
            }
        }

        fn resolve(device: &str) -> Result<CameraIndex, String> {
            if let Ok(i) = device.parse::<u32>() {
                return Ok(CameraIndex::Index(i));
            }
            let devices =
                query(ApiBackend::Auto).map_err(|e| format!("cannot list cameras: {e}"))?;
            let wanted = device.to_lowercase();
            devices
                .iter()
                .find(|d| d.human_name().to_lowercase().contains(&wanted))
                .map(|d| d.index().clone())
                .ok_or_else(|| {
                    let names: Vec<String> = devices.iter().map(|d| d.human_name()).collect();
                    format!("no camera matches `{device}`; available: {names:?}")
                })
        }
    }
}
