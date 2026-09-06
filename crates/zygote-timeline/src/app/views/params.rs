//! The parameter list: one row per parameter with its typed control, ghost marker, mod chip and override state.

use super::*;

impl TimelineApp {
    pub(super) fn render_params(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let values = self.effective_values();
        let offsets = self.offsets();
        let mode = self.mode();
        let mut list = v_flex().gap_1().w_full();
        if self.params.is_empty() {
            list = list.child(
                div()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child("waiting for the renderer to describe its graph…"),
            );
        }
        // Image sources have no parameters, so they would otherwise never
        // appear here. Show what they feed into the graph.
        if let Some(graph) = &self.graph {
            for node in &graph.nodes {
                let Some(file) = &node.preview else { continue };
                let focused = match &self.selected_node {
                    Some(filter) => filter == &node.id,
                    None => true,
                };
                if !focused {
                    continue;
                }
                let large = self.selected_node.is_some();
                let (w, h) = if large { (320.0, 180.0) } else { (96.0, 54.0) };
                list = list.child(
                    v_flex()
                        .gap_1()
                        .px_2()
                        .pt_2()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_baseline()
                                .child(
                                    div()
                                        .text_sm()
                                        .text_color(theme.foreground)
                                        .child(node.id.to_string()),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.muted_foreground)
                                        .child(node.kind.clone()),
                                ),
                        )
                        .child(
                            img(PathBuf::from(file))
                                .w(px(w))
                                .h(px(h))
                                .rounded_md()
                                .border_1()
                                .border_color(theme.border)
                                .bg(theme.muted)
                                .object_fit(ObjectFit::Contain),
                        )
                        .when(large, |d| {
                            d.child(
                                div()
                                    .text_xs()
                                    .font_family("monospace")
                                    .text_color(theme.muted_foreground)
                                    .child(file.clone()),
                            )
                        }),
                );
            }
        }
        let mut last_node: Option<NodeId> = None;
        for (i, control) in self.params.iter().enumerate() {
            let path = control.desc.path.clone();
            if let Some(filter) = &self.selected_node
                && filter != &path.node
            {
                continue;
            }
            if last_node.as_ref() != Some(&path.node) {
                last_node = Some(path.node.clone());
                let kind = self
                    .graph
                    .as_ref()
                    .and_then(|g| g.nodes.iter().find(|n| n.id == path.node))
                    .map(|n| n.kind.clone())
                    .unwrap_or_else(|| control.desc.node_kind.clone());
                list = list.child(
                    h_flex()
                        .gap_2()
                        .items_baseline()
                        .px_2()
                        .pt_2()
                        .child(
                            div()
                                .text_sm()
                                .text_color(theme.foreground)
                                .child(path.node.to_string()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(kind),
                        ),
                );
            }
            let overridden = self.overrides.contains_key(&path);
            let in_cue = matches!(mode, Mode::Edit(id) if self.timeline.cue(id).is_some_and(|c| c.values.contains_key(&path)));
            let value = values
                .get(&path)
                .cloned()
                .unwrap_or_else(|| control.desc.value.clone());
            let label_color = if overridden {
                theme.warning
            } else if in_cue {
                theme.primary
            } else {
                theme.foreground
            };

            let widget: AnyElement = match (&control.desc.ty, &value) {
                (ParamType::Bool, v) => {
                    let checked = v.as_bool().unwrap_or(false);
                    let path = path.clone();
                    Switch::new(("switch", i))
                        .checked(checked)
                        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
                            this.set_value(&path, ParamValue::Bool(*checked), cx)
                        }))
                        .into_any_element()
                }
                (ParamType::Choice { options }, v) => {
                    let current = v.as_choice().unwrap_or("");
                    let mut row = h_flex().gap_1().flex_wrap();
                    for (j, option) in options.iter().enumerate() {
                        let selected = option == current;
                        let path = path.clone();
                        let option_value = option.clone();
                        row = row.child(
                            Button::new(("choice", (i * 64 + j) as u64))
                                .xsmall()
                                .label(option.clone())
                                .when(selected, |b| b.primary())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_value(
                                        &path,
                                        ParamValue::Choice(option_value.clone()),
                                        cx,
                                    )
                                })),
                        );
                    }
                    row.into_any_element()
                }
                (ParamType::Color, v) => {
                    let c = v.as_color().unwrap_or([1.0; 4]);
                    let swatch = Rgba {
                        r: c[0],
                        g: c[1],
                        b: c[2],
                        a: 1.0,
                    };
                    let mut row = h_flex().gap_2().items_center().flex_1();
                    row = row.child(
                        div()
                            .w(px(22.))
                            .h(px(22.))
                            .rounded_sm()
                            .border_1()
                            .border_color(theme.border)
                            .bg(swatch),
                    );
                    for (slider, name) in control.sliders.iter().zip(["r", "g", "b"]) {
                        row = row
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(name),
                            )
                            .child(Slider::new(slider).flex_1());
                    }
                    row.into_any_element()
                }
                (ParamType::Vec2 { .. }, _) => {
                    let mut row = h_flex().gap_2().items_center().flex_1();
                    for (slider, name) in control.sliders.iter().zip(["x", "y"]) {
                        row = row
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(name),
                            )
                            .child(Slider::new(slider).flex_1());
                    }
                    row.into_any_element()
                }
                (ParamType::Float { .. } | ParamType::Int { .. }, v) => {
                    match control.sliders.first() {
                        Some(slider) => {
                            // Ghost marker: where the value actually is once modulation is added.
                            let ghost = offsets
                                .iter()
                                .find(|(p, _)| p == &path)
                                .map(|(_, offset)| {
                                    control.desc.ty.conform(&apply_offset(v, *offset))
                                })
                                .and_then(|g| {
                                    let (min, max) = match &control.desc.ty {
                                        ParamType::Float { min, max } => (*min, *max),
                                        ParamType::Int { min, max } => (*min as f32, *max as f32),
                                        _ => (0.0, 1.0),
                                    };
                                    g.as_float().map(|g| {
                                        ((g - min) / (max - min).max(1e-6)).clamp(0.0, 1.0)
                                    })
                                });
                            div()
                                .relative()
                                .flex_1()
                                .child(Slider::new(slider).w_full())
                                .when_some(ghost, |d, frac| {
                                    d.child(
                                        div()
                                            .absolute()
                                            .top(px(-3.))
                                            .left(relative(frac))
                                            .ml(px(-1.))
                                            .w(px(2.))
                                            .h(px(22.))
                                            .rounded_sm()
                                            .bg(theme.primary.opacity(0.8)),
                                    )
                                })
                                .into_any_element()
                        }
                        None => div().into_any_element(),
                    }
                }
            };

            let reset_path = path.clone();
            let assignment = self.timeline.modulation.assignment(&path).cloned();
            let modulatable = Self::depth_span(&control.desc).is_some();
            let editor_open = self.mod_editor.as_ref() == Some(&path);
            let chip_path = path.clone();
            list = list.child(
                h_flex()
                    .id(("param-row", i))
                    .gap_2()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .when(overridden, |d| d.bg(theme.warning.opacity(0.08)))
                    // Capture phase so the slider underneath cannot swallow the double-click.
                    .capture_any_mouse_down(cx.listener(
                        move |this, ev: &MouseDownEvent, window, cx| {
                            if ev.click_count >= 2 {
                                this.reset_to_default(&reset_path, window, cx);
                                cx.stop_propagation();
                            }
                        },
                    ))
                    .child(
                        div()
                            .w(px(170.))
                            .pl_3()
                            .text_sm()
                            .truncate()
                            .text_color(label_color)
                            .child(path.param.clone()),
                    )
                    .child(div().flex_1().child(widget))
                    .child(
                        div()
                            .w(px(72.))
                            .text_sm()
                            .font_family("monospace")
                            .text_color(theme.muted_foreground)
                            .child(value.to_string()),
                    )
                    .child(
                        Button::new(("mod", i))
                            .xsmall()
                            .label(match &assignment {
                                Some(a) => format!("{} ±{:.2}", a.source, a.depth.abs()),
                                None => "mod".to_owned(),
                            })
                            .when(assignment.is_some() || editor_open, |b| b.primary())
                            .disabled(!modulatable)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.toggle_mod_editor(&chip_path, cx)
                            })),
                    )
                    .child(
                        Button::new(("release", i))
                            .xsmall()
                            .label(if overridden {
                                "live ✕"
                            } else if in_cue {
                                "in cue"
                            } else {
                                "default"
                            })
                            .disabled(!overridden)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.release_override(&path, window, cx)
                            })),
                    ),
            );

            if editor_open {
                let mut sources = h_flex().gap_1().items_center().flex_wrap();
                let none_path = control.desc.path.clone();
                sources = sources.child(
                    Button::new(("mod-none", i))
                        .xsmall()
                        .label("none")
                        .when(assignment.is_none(), |b| b.primary())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.assign_source(&none_path, None, cx)
                        })),
                );
                for (j, source) in self.timeline.modulation.sources.iter().enumerate() {
                    let sp = control.desc.path.clone();
                    let sid = source.id.clone();
                    let selected = assignment.as_ref().is_some_and(|a| a.source == source.id);
                    sources = sources.child(
                        Button::new(("mod-src", (i * 64 + j) as u64))
                            .xsmall()
                            .label(source.id.clone())
                            .when(selected, |b| b.primary())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.assign_source(&sp, Some(sid.clone()), cx)
                            })),
                    );
                }
                if self.timeline.modulation.sources.is_empty() {
                    sources = sources.child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("add an LFO or envelope in the rail first"),
                    );
                }
                let mut editor = h_flex()
                    .gap_2()
                    .items_center()
                    .ml(px(190.))
                    .mr_2()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .bg(theme.primary.opacity(0.06))
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("source"),
                    )
                    .child(sources);
                if let Some((p, slider, _)) = &self.depth_slider
                    && p == &control.desc.path
                {
                    editor = editor
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child("depth ±"),
                        )
                        .child(Slider::new(slider).flex_1().min_w(px(160.)));
                }
                list = list.child(editor);
            }
        }
        list.into_any_element()
    }
}
