//! The node graph viewport: a read-only, pannable picture of the renderer's graph that doubles as the parameter filter.

use super::*;

impl TimelineApp {
    pub(super) fn graph_mouse_down(&mut self, ev: &MouseDownEvent, cx: &mut Context<Self>) {
        self.graph_drag = Some((ev.position, self.graph_pan));
        cx.notify();
    }

    pub(super) fn graph_mouse_move(&mut self, ev: &MouseMoveEvent, cx: &mut Context<Self>) {
        if let Some((start, pan)) = self.graph_drag {
            let dx: f32 = (ev.position.x - start.x).into();
            let dy: f32 = (ev.position.y - start.y).into();
            self.graph_pan = point(pan.x + dx, pan.y + dy);
            cx.notify();
        }
    }

    pub(super) fn graph_mouse_up(&mut self, cx: &mut Context<Self>) {
        self.graph_drag = None;
        cx.notify();
    }

    pub(super) fn graph_scroll(&mut self, ev: &ScrollWheelEvent, cx: &mut Context<Self>) {
        let delta = ev.delta.pixel_delta(px(24.));
        let dx: f32 = delta.x.into();
        let dy: f32 = delta.y.into();
        if ev.modifiers.shift {
            self.graph_pan = point(self.graph_pan.x + dy, self.graph_pan.y + dx);
        } else {
            self.graph_pan = point(self.graph_pan.x + dx + dy, self.graph_pan.y);
        }
        cx.notify();
    }

    pub(super) fn graph_fit(&mut self, content: (f32, f32), cx: &mut Context<Self>) {
        let viewport = self.graph_viewport.get();
        let vw: f32 = viewport.size.width.into();
        let vh: f32 = viewport.size.height.into();
        if vw <= 0.0 {
            return;
        }
        let x = if content.0 <= vw {
            (vw - content.0) / 2.0
        } else {
            0.0
        };
        let y = ((vh - content.1) / 2.0).max(0.0);
        self.graph_pan = point(x, y);
        self.graph_fit_pending = false;
        cx.notify();
    }

    /// Read-only picture of the renderer's node graph, pannable.
    pub(super) fn render_graph(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let Some(graph) = self.graph.clone() else {
            return div()
                .h(px(GRAPH_VIEW_H))
                .flex()
                .items_center()
                .text_sm()
                .text_color(theme.muted_foreground)
                .child("waiting for the renderer to send its graph…")
                .into_any_element();
        };
        let depths = structure_depths(&graph);
        let mut rows_in_col: BTreeMap<usize, usize> = BTreeMap::new();
        let mut placed: BTreeMap<NodeId, (usize, usize)> = BTreeMap::new();
        for node in &graph.nodes {
            let col = depths.get(&node.id).copied().unwrap_or(0);
            let row = rows_in_col.entry(col).or_insert(0);
            placed.insert(node.id.clone(), (col, *row));
            *row += 1;
        }
        let cols = rows_in_col.keys().max().map(|c| c + 1).unwrap_or(1);
        let rows = rows_in_col.values().copied().max().unwrap_or(1);
        let width = GRAPH_PAD * 2.0 + (cols as f32 + 1.0) * NODE_W + cols as f32 * COL_GAP;
        let height =
            GRAPH_PAD * 2.0 + rows as f32 * NODE_H + (rows as f32 - 1.0).max(0.0) * ROW_GAP;
        if self.graph_fit_pending {
            self.graph_fit((width, height), cx);
        }
        let pan = self.graph_pan;
        let theme = cx.theme();
        let pos = |col: usize, row: usize| -> (f32, f32) {
            (
                GRAPH_PAD + col as f32 * (NODE_W + COL_GAP),
                GRAPH_PAD + row as f32 * (NODE_H + ROW_GAP),
            )
        };

        let mut edges: Vec<((f32, f32), (f32, f32))> = Vec::new();
        for node in &graph.nodes {
            let Some(&(col, row)) = placed.get(&node.id) else {
                continue;
            };
            let (x, y) = pos(col, row);
            let slots = node.inputs.len().max(1) as f32;
            for (slot, link) in node.inputs.iter().enumerate() {
                let Some(input) = &link.from else { continue };
                let Some(&(icol, irow)) = placed.get(input) else {
                    continue;
                };
                let (ix, iy) = pos(icol, irow);
                let slot_y = y + NODE_H * (slot as f32 + 1.0) / (slots + 1.0);
                edges.push(((ix + NODE_W, iy + NODE_H / 2.0), (x, slot_y)));
            }
        }
        let window_col = cols;
        if let Some(&(col, row)) = placed.get(&graph.output) {
            let (x, y) = pos(col, row);
            let (wx, wy) = pos(window_col, 0);
            edges.push(((x + NODE_W, y + NODE_H / 2.0), (wx, wy + NODE_H / 2.0)));
        }

        let edge_color = theme.muted_foreground.opacity(0.7);
        let mut view = div()
            .relative()
            .w(px(width))
            .h(px(height))
            .flex_shrink_0()
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, _| {
                        for ((fx, fy), (tx, ty)) in edges {
                            let from = bounds.origin + point(px(fx), px(fy));
                            let to = bounds.origin + point(px(tx), px(ty));
                            let dx = (to.x - from.x) * 0.5;
                            let mut path = PathBuilder::stroke(px(1.5));
                            path.move_to(from);
                            path.cubic_bezier_to(
                                to,
                                point(from.x + dx, from.y),
                                point(to.x - dx, to.y),
                            );
                            if let Ok(path) = path.build() {
                                window.paint_path(path, edge_color);
                            }
                            let mut head = PathBuilder::fill();
                            head.move_to(to);
                            head.line_to(point(to.x - px(7.), to.y - px(4.)));
                            head.line_to(point(to.x - px(7.), to.y + px(4.)));
                            head.close();
                            if let Ok(head) = head.build() {
                                window.paint_path(head, edge_color);
                            }
                        }
                    },
                )
                .absolute()
                .inset_0(),
            );

        for (i, node) in graph.nodes.iter().enumerate() {
            let Some(&(col, row)) = placed.get(&node.id) else {
                continue;
            };
            let (x, y) = pos(col, row);
            let is_output = node.id == graph.output;
            let is_source = node.inputs.is_empty();
            let selected = self.selected_node.as_ref() == Some(&node.id);
            let select_id = node.id.clone();
            let thumb = node.preview.as_ref().map(|file| {
                img(PathBuf::from(file))
                    .w(px(NODE_H - 12.0))
                    .h(px(NODE_H - 12.0))
                    .flex_shrink_0()
                    .rounded_sm()
                    .object_fit(ObjectFit::Cover)
            });
            let mut slots = v_flex().justify_around().h_full().pr_1();
            for link in &node.inputs {
                let connected = link.from.is_some();
                slots = slots.child(
                    div()
                        .text_xs()
                        .text_color(if connected {
                            theme.muted_foreground
                        } else {
                            theme.muted_foreground.opacity(0.4)
                        })
                        .child(link.name.clone()),
                );
            }
            view = view.child(
                h_flex()
                    .id(("graph-node", i))
                    .absolute()
                    .left(px(x))
                    .top(px(y))
                    .w(px(NODE_W))
                    .h(px(NODE_H))
                    .px_2()
                    .gap_1()
                    .items_center()
                    .rounded_md()
                    .border_1()
                    .cursor_pointer()
                    .border_color(if selected || is_output {
                        theme.primary
                    } else {
                        theme.border
                    })
                    .when(selected, |d| d.border_2())
                    .bg(if selected {
                        theme.primary.opacity(0.14)
                    } else if is_source {
                        theme.muted
                    } else {
                        theme.background
                    })
                    .hover(|d| d.bg(theme.primary.opacity(0.08)))
                    .when(!node.enabled, |d| d.opacity(0.45))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.selected_node = if this.selected_node.as_ref() == Some(&select_id) {
                            None
                        } else {
                            Some(select_id.clone())
                        };
                        this.graph_node_clicked = true;
                        cx.notify();
                    }))
                    .when(!node.inputs.is_empty(), |d| d.child(slots))
                    .when_some(thumb, |d, thumb| d.child(thumb))
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .child(
                                div()
                                    .text_sm()
                                    .truncate()
                                    .text_color(theme.foreground)
                                    .child(node.id.to_string()),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .truncate()
                                    .text_color(theme.muted_foreground)
                                    .child(format!(
                                        "{}{}",
                                        node.kind,
                                        if node.feedback { " · feedback" } else { "" }
                                    )),
                            ),
                    ),
            );
        }
        let (wx, wy) = pos(window_col, 0);
        view = view.child(
            v_flex()
                .absolute()
                .left(px(wx))
                .top(px(wy))
                .w(px(NODE_W))
                .h(px(NODE_H))
                .px_2()
                .justify_center()
                .rounded_md()
                .border_1()
                .border_dashed()
                .border_color(theme.primary)
                .child(
                    div()
                        .text_sm()
                        .truncate()
                        .text_color(theme.foreground)
                        .child("output window"),
                )
                .child(
                    div()
                        .text_xs()
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child("3D quad · perspective camera"),
                ),
        );

        let viewport_cell = self.graph_viewport.clone();
        let content = (width, height);
        div()
            .id("graph-viewport")
            .relative()
            .w_full()
            .h(px(GRAPH_VIEW_H))
            .overflow_hidden()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.muted.opacity(0.2))
            .cursor_grab()
            .child(
                canvas(
                    move |bounds, _, _| viewport_cell.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .inset_0(),
            )
            .child(view.absolute().left(px(pan.x)).top(px(pan.y)))
            .child(
                h_flex()
                    .absolute()
                    .top_1()
                    .left_1()
                    .gap_1()
                    .items_center()
                    .child(match &self.selected_node {
                        Some(node) => Button::new("graph-show-all")
                            .xsmall()
                            .primary()
                            .icon(IconName::Close)
                            .label(format!("{node}"))
                            .tooltip("Showing this node's parameters. Click to show all.")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.selected_node = None;
                                cx.notify();
                            }))
                            .into_any_element(),
                        None => div()
                            .px_1p5()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child("all nodes · click one to focus its parameters")
                            .into_any_element(),
                    }),
            )
            .child(
                h_flex().absolute().top_1().right_1().gap_1().child(
                    Button::new("graph-fit")
                        .xsmall()
                        .ghost()
                        .icon(IconName::Maximize)
                        .tooltip("Fit the graph in view. Drag or scroll to pan.")
                        .on_click(cx.listener(move |this, _, _, cx| this.graph_fit(content, cx))),
                ),
            )
            .on_click(cx.listener(|this, _, _, cx| {
                if std::mem::take(&mut this.graph_node_clicked) {
                    return;
                }
                if this.selected_node.take().is_some() {
                    cx.notify();
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, ev: &MouseDownEvent, _, cx| this.graph_mouse_down(ev, cx)),
            )
            .on_mouse_move(
                cx.listener(|this, ev: &MouseMoveEvent, _, cx| this.graph_mouse_move(ev, cx)),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.graph_mouse_up(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _, _, cx| this.graph_mouse_up(cx)),
            )
            .on_scroll_wheel(
                cx.listener(|this, ev: &ScrollWheelEvent, _, cx| this.graph_scroll(ev, cx)),
            )
            .into_any_element()
    }
}

/// Longest-path depth per node of a UI structure (sources are 0).
fn structure_depths(graph: &GraphStructure) -> BTreeMap<NodeId, usize> {
    let mut depths: BTreeMap<NodeId, usize> = BTreeMap::new();
    for _ in 0..graph.nodes.len().max(1) {
        let mut changed = false;
        for node in &graph.nodes {
            let depth = node
                .inputs
                .iter()
                .filter_map(|l| l.from.as_ref())
                .filter_map(|from| depths.get(from))
                .map(|d| d + 1)
                .max()
                .unwrap_or(0);
            if depths.get(&node.id) != Some(&depth) {
                depths.insert(node.id.clone(), depth);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    depths
}
