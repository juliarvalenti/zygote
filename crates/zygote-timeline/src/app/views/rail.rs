//! The left rail: the modulation rack with live meters.

use super::*;

impl TimelineApp {
    /// Left rail: the modulation rack. Node filtering lives in the graph.
    pub(super) fn render_rail(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .id("rail")
            .w(px(RAIL_WIDTH))
            .flex_shrink_0()
            .h_full()
            .min_h_0()
            .overflow_y_scroll()
            .pr_3()
            .border_r_1()
            .border_color(theme.border)
            .child(self.render_rack(cx))
            .into_any_element()
    }

    /// Shared modulation sources with live meters.
    pub(super) fn render_rack(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let ctx = self.mod_ctx();
        let mut rack = v_flex().gap_2();
        rack = rack.child(
            h_flex()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .flex_1()
                        .child("MODULATION"),
                )
                .child(
                    Button::new("add-lfo")
                        .xsmall()
                        .icon(IconName::Plus)
                        .label("LFO")
                        .tooltip("Add a low-frequency oscillator")
                        .on_click(cx.listener(|this, _, _, cx| this.add_source(false, cx))),
                )
                .child(
                    Button::new("add-env")
                        .xsmall()
                        .icon(IconName::Plus)
                        .label("ENV")
                        .tooltip("Add an ADSR envelope, fired by a key")
                        .on_click(cx.listener(|this, _, _, cx| this.add_source(true, cx))),
                ),
        );
        for (i, source) in self.timeline.modulation.sources.iter().enumerate() {
            let value = source.sample(&ctx, &self.gates);
            let (frac, bipolar) = match source.kind {
                SourceKind::Lfo { .. } => ((value + 1.0) * 0.5, true),
                _ => (value, false),
            };
            let id = source.id.clone();
            let uses = self
                .timeline
                .modulation
                .assignments
                .iter()
                .filter(|a| a.source == source.id)
                .count();
            let mut card = v_flex()
                .gap_1()
                .p_2()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.muted.opacity(0.25));
            let remove_id = id.clone();
            card = card.child(
                h_flex()
                    .items_center()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .flex_1()
                            .truncate()
                            .text_color(theme.foreground)
                            .child(source.id.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(format!("{uses}×")),
                    )
                    .child(
                        Button::new(("rm-source", i))
                            .xsmall()
                            .ghost()
                            .icon(IconName::Close)
                            .tooltip("Remove this source and its assignments")
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.remove_source(&remove_id, cx)
                            })),
                    ),
            );
            // Live meter.
            card = card.child(
                div()
                    .relative()
                    .w_full()
                    .h(px(6.))
                    .rounded_sm()
                    .bg(theme.muted)
                    .when(bipolar, |d| {
                        d.child(
                            div()
                                .absolute()
                                .top_0()
                                .bottom_0()
                                .left(relative(0.5))
                                .w(px(1.))
                                .bg(theme.border),
                        )
                    })
                    .child(
                        div()
                            .absolute()
                            .top(px(-2.))
                            .left(relative(frac.clamp(0.0, 1.0)))
                            .ml(px(-3.))
                            .w(px(6.))
                            .h(px(10.))
                            .rounded_sm()
                            .bg(theme.primary),
                    ),
            );
            let sliders = self
                .source_sliders
                .get(&source.id)
                .map(|(s, _)| s.clone())
                .unwrap_or_default();
            match &source.kind {
                SourceKind::Lfo { shape, .. } => {
                    let mut shapes = h_flex().gap_1();
                    for (j, candidate) in LfoShape::ALL.iter().enumerate() {
                        let id = id.clone();
                        let c = *candidate;
                        shapes = shapes.child(
                            Button::new(("shape", (i * 8 + j) as u64))
                                .xsmall()
                                .label(c.label())
                                .when(*shape == c, |b| b.primary())
                                .on_click(
                                    cx.listener(move |this, _, _, cx| this.set_shape(&id, c, cx)),
                                ),
                        );
                    }
                    card = card.child(shapes);
                    for (slider, name) in sliders.iter().zip(["rate", "phase"]) {
                        card = card.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(36.))
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(name),
                                )
                                .child(Slider::new(slider).flex_1()),
                        );
                    }
                }
                SourceKind::Envelope { trigger, .. } => {
                    let bound = self
                        .key_for_action(&KeyAction::Trigger {
                            trigger: trigger.clone(),
                        })
                        .map(|k| format!("key `{k}`"))
                        .unwrap_or_else(|| "no key".to_owned());
                    let learning = self.learning == Some(LearnTarget::Trigger(trigger.clone()));
                    let t_learn = trigger.clone();
                    let t_down = trigger.clone();
                    let t_up = trigger.clone();
                    card = card.child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .text_xs()
                                    .flex_1()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(bound),
                            )
                            .child(
                                Button::new(("learn", i))
                                    .xsmall()
                                    .label(if learning { "press a key…" } else { "Learn" })
                                    .tooltip("Bind a key that fires this envelope")
                                    .when(learning, |b| b.primary())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.learning = Some(LearnTarget::Trigger(t_learn.clone()));
                                        cx.notify();
                                    })),
                            )
                            .child(
                                div()
                                    .id(("fire", i))
                                    .px_2()
                                    .py_0p5()
                                    .rounded_md()
                                    .border_1()
                                    .border_color(theme.border)
                                    .text_xs()
                                    .cursor_pointer()
                                    .child("hold")
                                    .on_mouse_down(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.gate(&t_down, true, cx)
                                        }),
                                    )
                                    .on_mouse_up(
                                        MouseButton::Left,
                                        cx.listener(move |this, _, _, cx| {
                                            this.gate(&t_up, false, cx)
                                        }),
                                    ),
                            ),
                    );
                    for (slider, name) in sliders.iter().zip(["A", "D", "S", "R"]) {
                        card = card.child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(
                                    div()
                                        .w(px(36.))
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(name),
                                )
                                .child(Slider::new(slider).flex_1()),
                        );
                    }
                }
                _ => {
                    card = card.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(source.label()),
                    );
                }
            }
            rack = rack.child(card);
        }
        if self.timeline.modulation.sources.is_empty() {
            rack = rack.child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child("No sources yet. Add an LFO or an envelope, then assign it from a parameter's mod chip."),
            );
        }
        rack.into_any_element()
    }
}
