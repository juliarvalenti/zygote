//! The UI side of the wire protocol: what to do with each message the renderer sends.

use super::*;

impl TimelineApp {
    pub(super) fn poll_network(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut installed = None;
        let messages: Vec<Message> = match &mut self.sender {
            Some(sender) => sender.poll(),
            None => Vec::new(),
        };
        for msg in messages {
            self.last_heard = Some(Instant::now());
            if self.quiet {
                self.quiet = false;
                if let Some(sender) = &self.sender {
                    let _ = sender.send(&Message::hello("zygote-timeline"));
                }
                self.last_hello = Instant::now();
            }
            match msg {
                Message::Structure {
                    protocol,
                    structure,
                } => {
                    self.note_protocol(protocol);
                    self.graph = Some(structure);
                    self.graph_fit_pending = true;
                }
                Message::Pong { protocol } => self.note_protocol(protocol),
                Message::Describe {
                    protocol,
                    graph,
                    chunk,
                    chunks,
                    params,
                } => {
                    self.note_protocol(protocol);
                    if chunk == 0 {
                        self.describe_buffer.clear();
                    }
                    self.describe_buffer.extend(params);
                    if chunk + 1 == chunks {
                        let was_described = self.described;
                        self.described = true;
                        self.status = format!(
                            "connected · graph `{graph}` · {} parameters{}",
                            self.describe_buffer.len(),
                            match self.protocol_mismatch {
                                Some(p) => format!(
                                    " · protocol mismatch (renderer {p}, ui {PROTOCOL_VERSION})"
                                ),
                                None => String::new(),
                            }
                        );
                        let fresh = std::mem::take(&mut self.describe_buffer);
                        let same = was_described
                            && self.params.len() == fresh.len()
                            && self
                                .params
                                .iter()
                                .zip(&fresh)
                                .all(|(c, d)| c.desc.path == d.path && c.desc.ty == d.ty);
                        installed = Some((fresh, same));
                    }
                }
                _ => {}
            }
        }
        if let Some((descriptors, same)) = installed {
            if self.timeline_is_default {
                let fits = self
                    .timeline
                    .cues
                    .iter()
                    .flat_map(|c| c.values.keys())
                    .any(|path| descriptors.iter().any(|d| &d.path == path));
                if !fits {
                    // Start from one cue holding the graph's defaults, parked on it.
                    let mut timeline = Timeline::new(8.0);
                    let values = descriptors
                        .iter()
                        .map(|d| (d.path.clone(), d.value.clone()))
                        .collect();
                    let id = timeline.add_cue(0.0, Transition::Cut, values);
                    self.timeline = timeline;
                    self.selected = Some(id);
                    self.playhead = 0.0;
                    self.overrides.clear();
                    self.status =
                        "new timeline: cue 1 holds the graph defaults; move sliders to edit it"
                            .to_owned();
                }
                self.timeline_is_default = false;
            }
            if !same {
                self.install_params(descriptors, window, cx);
            }
            self.sent.clear();
            self.push_values();
            self.send_modulation();
            self.resend_gates();
        } else if !self.described
            && self.params.is_empty()
            && self.started.elapsed() > DESCRIBE_TIMEOUT
        {
            self.status = format!(
                "{} · renderer not answering, using built-in first-pass parameter list",
                self.status
            );
            let library = NodeLibrary::builtin();
            let fallback = Graph::first_pass();
            self.install_params(fallback.describe_params(&library), window, cx);
            self.graph = Some(fallback.structure(&library));
            self.graph_fit_pending = true;
            self.described = true;
        }

        if self.last_hello.elapsed() >= HELLO_INTERVAL
            && let Some(sender) = &self.sender
        {
            let msg = if self.described && !self.quiet {
                Message::Ping
            } else {
                Message::hello("zygote-timeline")
            };
            let _ = sender.send(&msg);
            self.last_hello = Instant::now();
        }
        let silent = self
            .last_heard
            .is_some_and(|t| t.elapsed() > HELLO_INTERVAL * 3);
        if silent && !self.quiet {
            self.quiet = true;
            self.sent.clear();
            self.status = "renderer went quiet · waiting for it to come back".to_owned();
            cx.notify();
        }
    }

    pub(super) fn note_protocol(&mut self, protocol: u32) {
        self.protocol_mismatch = (protocol != PROTOCOL_VERSION).then_some(protocol);
    }
}
