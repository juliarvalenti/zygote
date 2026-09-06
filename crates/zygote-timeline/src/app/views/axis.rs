//! The time axis: cue chips, the segment lane, the playhead, and scrubbing.

use super::*;

impl TimelineApp {
    pub(super) fn time_at(&self, x: Pixels) -> f32 {
        let bounds = self.axis_bounds.get();
        let width: f32 = bounds.size.width.into();
        if width <= 0.0 {
            return 0.0;
        }
        let local: f32 = (x - bounds.origin.x).into();
        (local / width).clamp(0.0, 1.0) * self.timeline.duration
    }

    pub(super) fn axis_mouse_down(
        &mut self,
        ev: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.drag = Drag::Playhead;
        let t = self.time_at(ev.position.x);
        self.seek(t, window, cx);
    }

    pub(super) fn cue_mouse_down(&mut self, id: u32, window: &mut Window, cx: &mut Context<Self>) {
        self.drag = Drag::Cue(id);
        self.go_to_cue(id, window, cx);
        cx.stop_propagation();
    }

    pub(super) fn axis_mouse_move(
        &mut self,
        ev: &MouseMoveEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let t = self.time_at(ev.position.x);
        match self.drag {
            Drag::None => {}
            Drag::Playhead => self.seek(t, window, cx),
            Drag::Cue(id) => {
                self.timeline.move_cue(id, t);
                self.seek(t, window, cx);
            }
        }
    }

    pub(super) fn axis_mouse_up(&mut self, cx: &mut Context<Self>) {
        self.drag = Drag::None;
        cx.notify();
    }

    /// Time axis: tick row, cue chips on a lane, segment lane showing ramps
    /// and steps between cues, and the playhead across everything.
    pub(super) fn render_axis(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let duration = self.timeline.duration.max(1e-3);
        let bounds_cell = self.axis_bounds.clone();
        let primary = theme.primary;
        let muted = theme.muted_foreground;
        let border = theme.border;
        let danger = theme.danger;
        let selected = self.selected;
        let playhead = self.playhead;

        // Geometry (pixels from the top of the axis).
        let tick_h = 18.0;
        let chip_top = 24.0;
        let chip_h = 22.0;
        let seg_top = chip_top + chip_h + 10.0;
        let seg_bottom = AXIS_HEIGHT - 8.0;

        let cues: Vec<(f32, Transition)> = self
            .timeline
            .cues
            .iter()
            .map(|c| (c.time, c.transition))
            .collect();
        let segment_color = muted.opacity(0.8);

        let mut axis = div()
            .relative()
            .w_full()
            .h(px(AXIS_HEIGHT))
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(theme.muted.opacity(0.35))
            .overflow_hidden()
            .child(
                canvas(
                    move |bounds, _, _| bounds_cell.set(bounds),
                    move |bounds, _, window, _| {
                        let w: f32 = bounds.size.width.into();
                        let ox = bounds.origin.x;
                        let oy = bounds.origin.y;
                        let x_at = |t: f32| ox + px(w * (t / duration).clamp(0.0, 1.0));

                        // Segment lane: hold at the bottom, ramp up to the next cue
                        // when it interpolates, jump when it cuts.
                        if let Some(first) = cues.first() {
                            let mut path = PathBuilder::stroke(px(1.5));
                            path.move_to(point(ox, oy + px(seg_bottom)));
                            path.line_to(point(x_at(first.0), oy + px(seg_bottom)));
                            for pair in cues.windows(2) {
                                let (a, (b_time, b_transition)) = (pair[0].0, pair[1]);
                                match b_transition {
                                    Transition::Interpolate => {
                                        path.move_to(point(x_at(a), oy + px(seg_bottom)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_top)));
                                        path.move_to(point(x_at(b_time), oy + px(seg_top)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_bottom)));
                                    }
                                    Transition::Cut => {
                                        path.move_to(point(x_at(a), oy + px(seg_bottom)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_bottom)));
                                        path.move_to(point(x_at(b_time), oy + px(seg_bottom)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_top)));
                                        path.move_to(point(x_at(b_time), oy + px(seg_top)));
                                        path.line_to(point(x_at(b_time), oy + px(seg_bottom)));
                                    }
                                }
                            }
                            if let Some(last) = cues.last() {
                                path.move_to(point(x_at(last.0), oy + px(seg_bottom)));
                                path.line_to(point(ox + px(w), oy + px(seg_bottom)));
                            }
                            if let Ok(path) = path.build() {
                                window.paint_path(path, segment_color);
                            }
                        }

                        // Playhead: line across the axis with a head at the top.
                        let px_x = x_at(playhead);
                        window.paint_quad(fill(
                            Bounds {
                                origin: point(px_x - px(1.), oy),
                                size: size(px(2.), px(AXIS_HEIGHT)),
                            },
                            danger,
                        ));
                        let mut head = PathBuilder::fill();
                        head.move_to(point(px_x - px(6.), oy));
                        head.line_to(point(px_x + px(6.), oy));
                        head.line_to(point(px_x, oy + px(8.)));
                        head.close();
                        if let Ok(head) = head.build() {
                            window.paint_path(head, danger);
                        }
                    },
                )
                .absolute()
                .inset_0(),
            );

        // Ticks.
        let step = if duration > 60.0 {
            10.0
        } else if duration > 20.0 {
            5.0
        } else {
            1.0
        };
        let mut t = 0.0;
        while t <= duration + 1e-3 {
            let frac = t / duration;
            axis = axis
                .child(
                    div()
                        .absolute()
                        .top_0()
                        .left(relative(frac))
                        .w(px(1.))
                        .h(px(6.))
                        .bg(muted.opacity(0.6)),
                )
                .child(
                    div()
                        .absolute()
                        .top(px(4.))
                        .left(relative(frac))
                        .ml(px(3.))
                        .text_xs()
                        .text_color(muted)
                        .child(format!("{t:.0}")),
                );
            t += step;
        }
        let _ = tick_h;

        // Cue chips.
        for cue in &self.timeline.cues {
            let frac = (cue.time / duration).clamp(0.0, 1.0);
            let is_selected = selected == Some(cue.id);
            let id = cue.id;
            let glyph = match cue.transition {
                Transition::Cut => "⌐",
                Transition::Interpolate => "⟋",
            };
            axis = axis.child(
                h_flex()
                    .id(("cue", id as usize))
                    .absolute()
                    .top(px(chip_top))
                    .left(relative(frac))
                    .ml(px(-6.))
                    .h(px(chip_h))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .border_color(if is_selected { primary } else { border })
                    .bg(if is_selected {
                        primary
                    } else {
                        theme.background
                    })
                    .cursor_pointer()
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_selected {
                                theme.primary_foreground
                            } else {
                                muted
                            })
                            .child(glyph),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(if is_selected {
                                theme.primary_foreground
                            } else {
                                theme.foreground
                            })
                            .child(cue.label.clone()),
                    )
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |this, _, window, cx| this.cue_mouse_down(id, window, cx)),
                    ),
            );
        }

        axis.id("axis")
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, window, cx| {
                    this.axis_mouse_down(ev, window, cx)
                }),
            )
            .on_mouse_move(cx.listener(|this, ev: &MouseMoveEvent, window, cx| {
                this.axis_mouse_move(ev, window, cx)
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.axis_mouse_up(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.axis_mouse_up(cx)),
            )
            .into_any_element()
    }
}
