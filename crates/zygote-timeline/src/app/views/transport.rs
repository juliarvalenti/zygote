//! Transport bar and cue bar.

use super::*;

impl TimelineApp {
    pub(super) fn render_transport(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let mode = self.mode();
        let (mode_label, mode_color) = match mode {
            Mode::Edit(id) => (format!("editing cue {id}"), theme.primary),
            Mode::Live => (
                if self.playing {
                    "live"
                } else {
                    "live · not on a cue"
                }
                .to_owned(),
                theme.warning,
            ),
        };
        h_flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(
                Button::new("play")
                    .small()
                    .primary()
                    .icon(if self.playing {
                        IconName::Pause
                    } else {
                        IconName::Play
                    })
                    .label(if self.playing { "Pause" } else { "Play" })
                    .tooltip_with_action("Play / pause", &TogglePlay, Some("Zygote"))
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_play(cx))),
            )
            .child(
                Button::new("stop")
                    .small()
                    .label("Stop")
                    .tooltip_with_action("Stop and return to the last cue", &Stop, Some("Zygote"))
                    .on_click(cx.listener(|this, _, window, cx| this.stop(window, cx))),
            )
            .child(
                Button::new("prev-cue")
                    .small()
                    .icon(IconName::ChevronLeft)
                    .tooltip_with_action("Previous cue", &PrevCue, Some("Zygote"))
                    .on_click(cx.listener(|this, _, window, cx| this.step_cue(false, window, cx))),
            )
            .child(
                Button::new("next-cue")
                    .small()
                    .icon(IconName::ChevronRight)
                    .tooltip_with_action("Next cue", &NextCue, Some("Zygote"))
                    .on_click(cx.listener(|this, _, window, cx| this.step_cue(true, window, cx))),
            )
            .child(
                Button::new("loop")
                    .small()
                    .icon(IconName::RotateCw)
                    .label("Loop")
                    .toggled(self.timeline.looping)
                    .when(self.timeline.looping, |b| b.primary())
                    .tooltip_with_action("Loop the timeline", &ToggleLoop, Some("Zygote"))
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.timeline.looping = !this.timeline.looping;
                        cx.notify();
                    })),
            )
            .child(
                div()
                    .px_2()
                    .font_family("monospace")
                    .text_color(theme.foreground)
                    .child(format!(
                        "{:6.2}s / {:.0}s",
                        self.playhead, self.timeline.duration
                    )),
            )
            .child(
                Button::new("dur-")
                    .small()
                    .label("−1s")
                    .tooltip("Shorten the timeline by a second")
                    .on_click(cx.listener(|this, _, _, cx| this.adjust_duration(-1.0, cx))),
            )
            .child(
                Button::new("dur+")
                    .small()
                    .label("+1s")
                    .tooltip("Lengthen the timeline by a second")
                    .on_click(cx.listener(|this, _, _, cx| this.adjust_duration(1.0, cx))),
            )
            .child(div().flex_1())
            .child(
                div()
                    .px_2()
                    .py_0p5()
                    .rounded_md()
                    .text_xs()
                    .bg(mode_color.opacity(0.12))
                    .text_color(mode_color)
                    .child(mode_label),
            )
            .child(
                Button::new("mode")
                    .small()
                    .label(if self.force_live {
                        "Edit cues"
                    } else {
                        "Force live"
                    })
                    .tooltip_with_action(
                        "Edit mode writes slider moves into the parked cue; live mode keeps them as overrides",
                        &ToggleLive,
                        Some("Zygote"),
                    )
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.force_live = !this.force_live;
                        cx.notify();
                    })),
            )
            .into_any_element()
    }

    pub(super) fn render_cue_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let selected = self.selected.and_then(|id| self.timeline.cue(id));
        let theme = cx.theme();
        h_flex()
            .gap_2()
            .items_center()
            .flex_wrap()
            .child(
                Button::new("add-cue")
                    .small()
                    .primary()
                    .icon(IconName::Plus)
                    .label("Add cue")
                    .tooltip_with_action("Add a cue at the playhead", &AddCue, Some("Zygote"))
                    .on_click(cx.listener(|this, _, _, cx| this.add_cue(cx))),
            )
            .child(
                Button::new("bake-cue")
                    .small()
                    .label("Bake live")
                    .tooltip("Write the current live overrides into the selected cue")
                    .disabled(selected.is_none() || self.overrides.is_empty())
                    .on_click(cx.listener(|this, _, _, cx| this.bake_into_selected(cx))),
            )
            .child(
                Button::new("toggle-transition")
                    .small()
                    .label(match selected.map(|c| c.transition) {
                        Some(Transition::Cut) => "Into cue: step",
                        Some(Transition::Interpolate) => "Into cue: ramp",
                        None => "Into cue: —",
                    })
                    .disabled(selected.is_none())
                    .tooltip("How values reach this cue: jump (step) or interpolate from the previous cue (ramp)")
                    .on_click(cx.listener(|this, _, _, cx| this.toggle_transition(cx))),
            )
            .child(
                Button::new("delete-cue")
                    .small()
                    .danger()
                    .icon(IconName::Close)
                    .label("Delete")
                    .tooltip("Delete the selected cue")
                    .disabled(selected.is_none())
                    .on_click(cx.listener(|this, _, _, cx| this.delete_selected(cx))),
            )
            .child(
                Button::new("learn-cue-key")
                    .small()
                    .label(match selected {
                        Some(cue) => match (
                            &self.learning,
                            self.key_for_action(&KeyAction::Cue { id: cue.id }),
                        ) {
                            (Some(LearnTarget::Cue(id)), _) if *id == cue.id => {
                                "press a key…".to_owned()
                            }
                            (_, Some(k)) => format!("Key: {k}"),
                            _ => "Learn key".to_owned(),
                        },
                        None => "Learn key".to_owned(),
                    })
                    .disabled(selected.is_none())
                    .tooltip("Bind a key that jumps to this cue")
                    .on_click(cx.listener(|this, _, _, cx| {
                        if let Some(id) = this.selected {
                            this.learning = Some(LearnTarget::Cue(id));
                            cx.notify();
                        }
                    })),
            )
            .child(div().flex_1())
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(match selected {
                        Some(cue) => format!(
                            "{} · {:.2}s · {} values",
                            cue.label,
                            cue.time,
                            cue.values.len()
                        ),
                        None => "no cue selected".to_owned(),
                    }),
            )
            .child(
                Button::new("save")
                    .small()
                    .label("Save")
                    .tooltip("Save the show file (cues, modulation, keys)")
                    .on_click(cx.listener(|this, _, _, cx| this.save(cx))),
            )
            .child(
                Button::new("load")
                    .small()
                    .label("Load")
                    .tooltip("Reload the show file from disk")
                    .on_click(cx.listener(|this, _, window, cx| this.load(window, cx))),
            )
            .into_any_element()
    }
}
