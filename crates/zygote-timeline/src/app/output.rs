//! Placing the renderer's output window: tiling beside this window and releasing it.

use super::*;

impl TimelineApp {
    /// Put the output window to the right of this one, 16:9, on the same display.
    pub(super) fn tile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(sender) = &self.sender else { return };
        let ui = window.bounds();
        let display = window.display(cx).map(|d| d.bounds()).unwrap_or(ui);
        let scale = window.scale_factor();
        let gap = px(12.);
        let x = ui.origin.x + ui.size.width + gap;
        let available_w = (display.origin.x + display.size.width - x - gap).max(px(320.));
        let available_h = (display.size.height - px(80.)).max(px(180.));
        let mut w = available_w;
        let mut h = w * 9.0 / 16.0;
        if h > available_h {
            h = available_h;
            w = h * 16.0 / 9.0;
        }
        let f = |v: Pixels| -> f32 { v.into() };
        // Bevy positions windows in physical pixels and sizes them in logical pixels.
        let bounds = OutputBounds {
            x: (f(x) * scale) as i32,
            y: (f(ui.origin.y) * scale) as i32,
            width: f(w) as u32,
            height: f(h) as u32,
        };
        let _ = sender.send(&Message::Arrange {
            bounds: Some(bounds),
        });
        self.tiled = true;
        self.status = format!(
            "output tiled at {}x{} next to this window",
            bounds.width, bounds.height
        );
        cx.notify();
    }

    pub(super) fn release_output(&mut self, cx: &mut Context<Self>) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(&Message::Arrange { bounds: None });
        }
        self.tiled = false;
        self.status = "output window released".to_owned();
        cx.notify();
    }
}
