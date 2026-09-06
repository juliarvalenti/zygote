//! Keys and gestures panel.

use super::*;

impl TimelineApp {
    /// Keys and gestures, shown on demand from the header's info button.
    pub(super) fn render_help(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (border, key_bg, key_fg, fg, muted) = (
            theme.border,
            theme.muted,
            theme.foreground,
            theme.foreground,
            theme.muted_foreground,
        );
        let key = move |k: &str| {
            div()
                .px_1p5()
                .py_0p5()
                .rounded_sm()
                .bg(key_bg)
                .text_xs()
                .font_family("monospace")
                .text_color(key_fg)
                .child(k.to_owned())
        };
        let row = move |keys: &[&str], what: &str| {
            let mut chips = h_flex().gap_1().flex_shrink_0().w(px(96.));
            for k in keys {
                chips = chips.child(key(k));
            }
            h_flex().w_full().gap_2().items_start().child(chips).child(
                div()
                    .flex_1()
                    .min_w_0()
                    .text_xs()
                    .text_color(fg)
                    .child(what.to_owned()),
            )
        };
        let column = move |title: &str, rows: Vec<AnyElement>| {
            let mut col = v_flex().gap_1().flex_1().min_w(px(280.)).child(
                div()
                    .text_xs()
                    .text_color(muted)
                    .pb_1()
                    .child(title.to_owned()),
            );
            for r in rows {
                col = col.child(r);
            }
            col
        };
        h_flex()
            .gap_4()
            .flex_wrap()
            .items_start()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(border)
            .child(column(
                "TRANSPORT",
                vec![
                    row(&["space"], "play / pause").into_any_element(),
                    row(&["esc"], "stop, back to the last cue").into_any_element(),
                    row(&["home"], "go to start").into_any_element(),
                    row(&["[", "]"], "previous / next cue").into_any_element(),
                    row(&["⏎"], "add a cue at the playhead").into_any_element(),
                    row(&["l"], "toggle loop").into_any_element(),
                    row(&["e"], "toggle edit / live").into_any_element(),
                ],
            ))
            .child(column(
                "EDITING",
                vec![
                    row(&["double-click"], "reset a parameter to its default").into_any_element(),
                    row(&["click node"], "focus one node's parameters").into_any_element(),
                    row(&["drag", "scroll"], "pan the graph").into_any_element(),
                    row(&["mod"], "assign an LFO or envelope, set depth").into_any_element(),
                    row(&["Learn"], "bind a key to a cue or envelope").into_any_element(),
                ],
            ))
            .child(column(
                "OUTPUT WINDOW",
                vec![
                    row(&["t", "shift-t"], "tile beside this window / pop out").into_any_element(),
                    row(&["scroll", "=", "-"], "zoom the view").into_any_element(),
                    row(&["drag", "arrows"], "pan the view").into_any_element(),
                    row(&["right-drag"], "orbit the quad").into_any_element(),
                    row(&["home", "0"], "reset the view").into_any_element(),
                    row(&["s"], "save a screenshot").into_any_element(),
                    row(&["esc"], "quit the renderer").into_any_element(),
                ],
            ))
            .into_any_element()
    }
}
