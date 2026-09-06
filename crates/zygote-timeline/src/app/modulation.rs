//! Modulation sources, assignments and gates: the model side of the rack.

use super::*;

impl TimelineApp {
    pub(super) fn mod_ctx(&self) -> ModContext {
        ModContext {
            time: self.playhead,
            ..Default::default()
        }
    }

    /// Offsets the renderer is adding right now, computed from the same
    /// definition and clock so the ghost markers match the picture.
    pub(super) fn offsets(&self) -> Vec<(ParamPath, f32)> {
        self.timeline
            .modulation
            .offsets(&self.mod_ctx(), &self.gates)
    }

    pub(super) fn send_modulation(&mut self) {
        if let Some(sender) = &self.sender {
            let _ = sender.send(&Message::Modulation {
                modulation: self.timeline.modulation.clone(),
            });
        }
        self.mod_dirty = false;
    }

    pub(super) fn resend_gates(&self) {
        let Some(sender) = &self.sender else { return };
        for event in self.gates.events() {
            let _ = sender.send(&Message::Gate {
                event: event.clone(),
            });
        }
    }

    pub(super) fn gate(&mut self, trigger: &str, on: bool, cx: &mut Context<Self>) {
        let event = GateEvent {
            trigger: trigger.to_owned(),
            on,
            time: self.playhead,
        };
        self.gates.push(event.clone());
        if let Some(sender) = &self.sender {
            let _ = sender.send(&Message::Gate { event });
        }
        cx.notify();
    }

    pub(super) fn add_source(&mut self, envelope: bool, cx: &mut Context<Self>) {
        let source = if envelope {
            let id = self.timeline.modulation.next_id("env");
            let trigger = id.clone();
            ModSource::envelope(&id, &trigger)
        } else {
            let id = self.timeline.modulation.next_id("lfo");
            ModSource::lfo(&id, 0.25, LfoShape::Sine)
        };
        self.timeline.modulation.sources.push(source);
        self.ensure_source_sliders(cx);
        self.mod_dirty = true;
        cx.notify();
    }

    pub(super) fn remove_source(&mut self, id: &str, cx: &mut Context<Self>) {
        self.timeline.modulation.remove_source(id);
        self.source_sliders.remove(id);
        let trigger_keys: Vec<String> = self
            .timeline
            .keys
            .iter()
            .filter(|(_, a)| matches!(a, KeyAction::Trigger { trigger } if trigger == id))
            .map(|(k, _)| k.clone())
            .collect();
        for k in trigger_keys {
            self.timeline.keys.remove(&k);
        }
        self.mod_dirty = true;
        cx.notify();
    }

    pub(super) fn set_shape(&mut self, id: &str, shape: LfoShape, cx: &mut Context<Self>) {
        if let Some(source) = self.timeline.modulation.source_mut(id)
            && let SourceKind::Lfo { shape: s, .. } = &mut source.kind
        {
            *s = shape;
            self.mod_dirty = true;
        }
        cx.notify();
    }

    /// Sliders for every source that lacks them: LFO `[rate, phase]`,
    /// envelope `[attack, decay, sustain, release]`.
    pub(super) fn ensure_source_sliders(&mut self, cx: &mut Context<Self>) {
        let sources = self.timeline.modulation.sources.clone();
        for source in sources {
            if self.source_sliders.contains_key(&source.id) {
                continue;
            }
            let specs: Vec<(f32, f32, f32)> = match &source.kind {
                SourceKind::Lfo { rate_hz, phase, .. } => {
                    vec![(0.02, 4.0, *rate_hz), (0.0, 1.0, *phase)]
                }
                SourceKind::Envelope { adsr, .. } => vec![
                    (0.0, 2.0, adsr.attack),
                    (0.0, 3.0, adsr.decay),
                    (0.0, 1.0, adsr.sustain),
                    (0.0, 4.0, adsr.release),
                ],
                _ => Vec::new(),
            };
            let mut sliders = Vec::new();
            let mut subs = Vec::new();
            for (index, (min, max, value)) in specs.into_iter().enumerate() {
                let slider = cx.new(|_| {
                    SliderState::new()
                        .min(min)
                        .max(max)
                        .step((max - min) / 400.0)
                        .default_value(value)
                });
                let id = source.id.clone();
                subs.push(
                    cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
                        if let SliderEvent::Change(v) = event {
                            this.set_source_field(&id, index, v.start(), cx);
                        }
                    }),
                );
                sliders.push(slider);
            }
            self.source_sliders
                .insert(source.id.clone(), (sliders, subs));
        }
    }

    pub(super) fn set_source_field(
        &mut self,
        id: &str,
        index: usize,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        if let Some(source) = self.timeline.modulation.source_mut(id) {
            match &mut source.kind {
                SourceKind::Lfo { rate_hz, phase, .. } => match index {
                    0 => *rate_hz = value,
                    _ => *phase = value,
                },
                SourceKind::Envelope { adsr, .. } => match index {
                    0 => adsr.attack = value,
                    1 => adsr.decay = value,
                    2 => adsr.sustain = value,
                    _ => adsr.release = value,
                },
                _ => {}
            }
            self.mod_dirty = true;
        }
        cx.notify();
    }

    /// Bipolar depth range for a parameter: its whole span either way.
    pub(super) fn depth_span(desc: &ParamDescriptor) -> Option<f32> {
        match &desc.ty {
            ParamType::Float { min, max } => Some((max - min).abs().max(1e-6)),
            ParamType::Int { min, max } => Some(((max - min).abs() as f32).max(1.0)),
            _ => None,
        }
    }

    pub(super) fn toggle_mod_editor(&mut self, path: &ParamPath, cx: &mut Context<Self>) {
        if self.mod_editor.as_ref() == Some(path) {
            self.mod_editor = None;
            self.depth_slider = None;
            cx.notify();
            return;
        }
        let Some(desc) = self
            .params
            .iter()
            .find(|c| &c.desc.path == path)
            .map(|c| c.desc.clone())
        else {
            return;
        };
        let Some(span) = Self::depth_span(&desc) else {
            return;
        };
        let depth = self
            .timeline
            .modulation
            .assignment(path)
            .map(|a| a.depth)
            .unwrap_or(0.0);
        let slider = cx.new(|_| {
            SliderState::new()
                .min(-span)
                .max(span)
                .step(span / 250.0)
                .default_value(depth)
        });
        let p = path.clone();
        let sub = cx.subscribe(&slider, move |this, _, event: &SliderEvent, cx| {
            if let SliderEvent::Change(v) = event {
                this.set_depth(&p, v.start(), cx);
            }
        });
        self.depth_slider = Some((path.clone(), slider, sub));
        self.mod_editor = Some(path.clone());
        cx.notify();
    }

    pub(super) fn set_depth(&mut self, path: &ParamPath, depth: f32, cx: &mut Context<Self>) {
        let source = self
            .timeline
            .modulation
            .assignment(path)
            .map(|a| a.source.clone())
            .or_else(|| {
                self.timeline
                    .modulation
                    .sources
                    .first()
                    .map(|s| s.id.clone())
            });
        if let Some(source) = source {
            self.timeline.modulation.assign(path, Some(&source), depth);
            self.mod_dirty = true;
        }
        cx.notify();
    }

    pub(super) fn assign_source(
        &mut self,
        path: &ParamPath,
        source: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let existing = self.timeline.modulation.assignment(path).map(|a| a.depth);
        let depth = existing.unwrap_or_else(|| {
            self.params
                .iter()
                .find(|c| &c.desc.path == path)
                .and_then(|c| Self::depth_span(&c.desc))
                .map(|span| span * 0.25)
                .unwrap_or(0.0)
        });
        self.timeline
            .modulation
            .assign(path, source.as_deref(), depth);
        self.mod_dirty = true;
        cx.notify();
    }
}
