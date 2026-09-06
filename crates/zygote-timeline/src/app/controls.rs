//! One control per parameter: building sliders from descriptors, edits, overrides and keeping the widgets in sync with the resolved values.

use super::*;

impl TimelineApp {
    pub(super) fn install_params(
        &mut self,
        descriptors: Vec<ParamDescriptor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let values = self.effective_values();
        self.params.clear();
        for desc in descriptors {
            let current = values
                .get(&desc.path)
                .cloned()
                .unwrap_or_else(|| desc.value.clone());
            // (min, max, step, value) per slider component.
            let ranges: Vec<(f32, f32, f32, f32)> = match (&desc.ty, &current) {
                (ParamType::Float { min, max }, v) => {
                    vec![(
                        *min,
                        *max,
                        (max - min).abs().max(1e-6) / 1000.0,
                        v.as_float().unwrap_or(*min),
                    )]
                }
                (ParamType::Int { min, max }, v) => {
                    vec![(
                        *min as f32,
                        *max as f32,
                        1.0,
                        v.as_int().unwrap_or(*min) as f32,
                    )]
                }
                (ParamType::Vec2 { min, max }, v) => {
                    let xy = v.as_vec2().unwrap_or([0.0; 2]);
                    let step = (max - min).abs().max(1e-6) / 1000.0;
                    vec![(*min, *max, step, xy[0]), (*min, *max, step, xy[1])]
                }
                (ParamType::Color, v) => {
                    let c = v.as_color().unwrap_or([1.0; 4]);
                    vec![
                        (0.0, 1.0, 0.001, c[0]),
                        (0.0, 1.0, 0.001, c[1]),
                        (0.0, 1.0, 0.001, c[2]),
                    ]
                }
                _ => Vec::new(),
            };
            let mut sliders = Vec::new();
            let mut subscriptions = Vec::new();
            for (component, (min, max, step, value)) in ranges.into_iter().enumerate() {
                let slider = cx.new(|_| {
                    SliderState::new()
                        .min(min)
                        .max(max)
                        .step(step)
                        .default_value(value)
                });
                let path = desc.path.clone();
                subscriptions.push(cx.subscribe(
                    &slider,
                    move |this, _, event: &SliderEvent, cx| {
                        if let SliderEvent::Change(value) = event {
                            this.on_component(&path, component, value.start(), cx);
                        }
                    },
                ));
                sliders.push(slider);
            }
            self.params.push(ParamControl {
                desc,
                sliders,
                _subscriptions: subscriptions,
            });
        }
        self.sync_sliders(window, cx);
        cx.notify();
    }

    /// A slider moved: one component of a parameter changed.
    pub(super) fn on_component(
        &mut self,
        path: &ParamPath,
        component: usize,
        value: f32,
        cx: &mut Context<Self>,
    ) {
        let control = self.params.iter().find(|c| &c.desc.path == path);
        let current = self
            .effective_values()
            .get(path)
            .cloned()
            .or_else(|| control.map(|c| c.desc.value.clone()));
        let Some(current) = current else { return };
        let next = match (control.map(|c| &c.desc.ty), current) {
            (Some(ParamType::Int { .. }), _) => ParamValue::Int(value.round() as i32),
            (_, ParamValue::Float(_)) => ParamValue::Float(value),
            (_, ParamValue::Int(_)) => ParamValue::Int(value.round() as i32),
            (_, ParamValue::Vec2(mut v)) => {
                if let Some(slot) = v.get_mut(component) {
                    *slot = value;
                }
                ParamValue::Vec2(v)
            }
            (_, ParamValue::Color(mut c)) => {
                if let Some(slot) = c.get_mut(component) {
                    *slot = value;
                }
                ParamValue::Color(c)
            }
            (_, other) => other,
        };
        self.set_value(path, next, cx);
    }

    /// Apply a control change according to the current mode.
    pub(super) fn set_value(
        &mut self,
        path: &ParamPath,
        value: ParamValue,
        cx: &mut Context<Self>,
    ) {
        match self.mode() {
            Mode::Edit(id) => {
                if let Some(cue) = self.timeline.cue_mut(id) {
                    cue.values.insert(path.clone(), value);
                }
                // The cue now carries the value; a stale override would hide it.
                self.overrides.remove(path);
            }
            Mode::Live => {
                self.overrides.insert(path.clone(), value);
            }
        }
        self.push_values();
        cx.notify();
    }

    /// Double-click: back to the declared default (into the cue when editing).
    pub(super) fn reset_to_default(
        &mut self,
        path: &ParamPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(default) = self
            .params
            .iter()
            .find(|c| &c.desc.path == path)
            .map(|c| c.desc.default.clone())
        else {
            return;
        };
        self.set_value(path, default, cx);
        self.sync_sliders(window, cx);
    }

    pub(super) fn release_override(
        &mut self,
        path: &ParamPath,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.overrides.remove(path);
        if !self.timeline.evaluate(self.playhead).contains_key(path)
            && let Some(sender) = &self.sender
        {
            let _ = sender.send(&Message::ClearParam { path: path.clone() });
            self.sent.remove(path);
        }
        self.push_values();
        self.sync_sliders(window, cx);
        cx.notify();
    }

    pub(super) fn release_all(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths: Vec<_> = self.overrides.keys().cloned().collect();
        for path in paths {
            self.release_override(&path, window, cx);
        }
    }

    /// Timeline values with manual overrides applied.
    pub(super) fn effective_values(&self) -> BTreeMap<ParamPath, ParamValue> {
        let mut values = self.timeline.evaluate(self.playhead);
        for (path, value) in &self.overrides {
            values.insert(path.clone(), value.clone());
        }
        values
    }

    /// Send whatever changed since the last push.
    pub(super) fn push_values(&mut self) {
        let Some(sender) = &self.sender else { return };
        let values = self.effective_values();
        let changed: Vec<(ParamPath, ParamValue)> = values
            .iter()
            .filter(|(path, value)| self.sent.get(*path) != Some(*value))
            .map(|(path, value)| (path.clone(), value.clone()))
            .collect();
        if changed.is_empty() {
            return;
        }
        for chunk in changed.chunks(32) {
            let _ = sender.send(&Message::SetParams {
                values: chunk.to_vec(),
            });
        }
        for (path, value) in changed {
            self.sent.insert(path, value);
        }
    }

    pub(super) fn sync_sliders(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let values = self.effective_values();
        for control in &self.params {
            let Some(value) = values.get(&control.desc.path) else {
                continue;
            };
            let components: Vec<f32> = match value {
                ParamValue::Float(v) => vec![*v],
                ParamValue::Int(v) => vec![*v as f32],
                ParamValue::Vec2(v) => v.to_vec(),
                ParamValue::Color(c) => c[..3].to_vec(),
                _ => Vec::new(),
            };
            for (slider, component) in control.sliders.iter().zip(components) {
                slider.update(cx, |slider, cx| {
                    if (slider.value().start() - component).abs() > 1e-6 {
                        slider.set_value(component, window, cx);
                    }
                });
            }
        }
    }
}
