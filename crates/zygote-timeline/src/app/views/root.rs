//! The window: header, rail, main column, status line, and the transport key actions.

use super::*;

impl Render for TimelineApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (background, foreground, muted) = {
            let theme = cx.theme();
            (theme.background, theme.foreground, theme.muted_foreground)
        };
        let header = h_flex()
            .items_center()
            .gap_2()
            .child(
                Button::new("projects")
                    .small()
                    .ghost()
                    .icon(IconName::PanelLeft)
                    .tooltip("Back to the project list (the renderer keeps running)")
                    .on_click(cx.listener(|_, _, _, cx| cx.emit(TimelineEvent::Projects))),
            )
            .child(div().text_lg().child("Zygote"))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .truncate()
                    .text_color(muted)
                    .child(
                        self.graph
                            .as_ref()
                            .map(|g| g.name.clone())
                            .unwrap_or_default(),
                    ),
            )
            .child(
                Button::new("tile")
                    .small()
                    .icon(IconName::PanelRight)
                    .label(if self.tiled { "Re-tile" } else { "Tile output" })
                    .disabled(self.sender.is_none())
                    .tooltip_with_action(
                        "Place the output window beside this one",
                        &TileOutput,
                        Some("Zygote"),
                    )
                    .on_click(cx.listener(|this, _, window, cx| this.tile(window, cx))),
            )
            .child(
                Button::new("release-output")
                    .small()
                    .icon(IconName::ExternalLink)
                    .label("Pop out")
                    .disabled(!self.tiled)
                    .tooltip_with_action(
                        "Let the output window float free (drag it to the projector)",
                        &PopOut,
                        Some("Zygote"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| this.release_output(cx))),
            )
            .child(
                Button::new("release-all")
                    .small()
                    .icon(IconName::Undo2)
                    .label("Release live")
                    .disabled(self.overrides.is_empty())
                    .tooltip("Drop every live override; parameters fall back to their cues")
                    .on_click(cx.listener(|this, _, window, cx| this.release_all(window, cx))),
            )
            .child(
                Button::new("help")
                    .small()
                    .ghost()
                    .icon(IconName::Info)
                    .toggled(self.show_help)
                    .when(self.show_help, |b| b.primary())
                    .tooltip("Keys and gestures")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.show_help = !this.show_help;
                        cx.notify();
                    })),
            );
        let help = if self.show_help {
            Some(self.render_help(cx))
        } else {
            None
        };
        let rail = self.render_rail(cx);
        let transport = self.render_transport(cx);
        let graph = self.render_graph(cx);
        let axis = self.render_axis(cx);
        let cue_bar = self.render_cue_bar(cx);
        let params = self.render_params(cx);

        let mut main = v_flex()
            .flex_1()
            .min_w_0()
            .h_full()
            .min_h_0()
            .gap_3()
            .child(transport)
            .child(graph);
        if let Some(help) = help {
            main = main.child(help);
        }
        main = main.child(axis).child(cue_bar).child(
            div()
                .id("params")
                .flex_1()
                .min_h_0()
                .overflow_y_scroll()
                .child(params),
        );

        v_flex()
            .id("root")
            .track_focus(&self.focus_handle)
            .key_context("Zygote")
            .capture_action(cx.listener(|this, _: &TogglePlay, _, cx| {
                this.toggle_play(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &Stop, window, cx| {
                if this.learning.take().is_some() {
                    cx.notify();
                } else {
                    this.stop(window, cx);
                }
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &GoToStart, window, cx| {
                this.seek(0.0, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &PrevCue, window, cx| {
                this.step_cue(false, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &NextCue, window, cx| {
                this.step_cue(true, window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &AddCue, _, cx| {
                this.add_cue(cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &ToggleLoop, _, cx| {
                this.timeline.looping = !this.timeline.looping;
                cx.notify();
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &ToggleLive, _, cx| {
                this.force_live = !this.force_live;
                cx.notify();
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &TileOutput, window, cx| {
                this.tile(window, cx);
                cx.stop_propagation();
            }))
            .capture_action(cx.listener(|this, _: &PopOut, _, cx| {
                this.release_output(cx);
                cx.stop_propagation();
            }))
            .on_key_down(
                cx.listener(|this, ev: &KeyDownEvent, window, cx| this.key_down(ev, window, cx)),
            )
            .on_key_up(cx.listener(|this, ev: &KeyUpEvent, _, cx| this.key_up(ev, cx)))
            .size_full()
            .bg(background)
            .text_color(foreground)
            .p_4()
            .gap_3()
            .child(header)
            .child(
                h_flex()
                    .flex_1()
                    .min_h_0()
                    .gap_4()
                    .items_start()
                    .child(rail)
                    .child(main),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .child(match &self.learning {
                        Some(LearnTarget::Trigger(t)) => {
                            format!("press a key to bind trigger `{t}` · esc cancels")
                        }
                        Some(LearnTarget::Cue(id)) => {
                            format!("press a key to bind cue {id} · esc cancels")
                        }
                        None => self.status.clone(),
                    }),
            )
    }
}
