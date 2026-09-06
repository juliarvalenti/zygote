//! Transport and cues: the clock tick, play/stop/seek, cue editing, and the show file on disk.

use super::*;

impl TimelineApp {
    pub(super) fn tick(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.auto_tile && !self.tiled && self.graph.is_some() {
            self.auto_tile = false;
            self.tile(window, cx);
        }
        let now = Instant::now();
        let dt = now.duration_since(self.last_tick).as_secs_f32();
        self.last_tick = now;

        self.poll_network(window, cx);

        if self.playing {
            let t = self.playhead + dt;
            if !self.timeline.looping && t >= self.timeline.duration {
                self.playhead = self.timeline.duration;
                self.playing = false;
            } else {
                self.playhead = self.timeline.wrap_time(t);
            }
        }

        if self.playing || self.drag != Drag::None {
            self.sync_sliders(window, cx);
        }
        self.push_values();
        if self.mod_dirty {
            self.send_modulation();
        }
        // The renderer's clock follows this. Paused → frozen picture.
        if let Some(sender) = &self.sender {
            let _ = sender.send(&Message::Transport {
                time: self.playhead,
                playing: self.playing,
            });
        }
        cx.notify();
    }

    pub(super) fn toggle_play(&mut self, cx: &mut Context<Self>) {
        self.playing = !self.playing;
        cx.notify();
    }

    pub(super) fn stop(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.playing = false;
        self.seek(0.0, window, cx);
        // Park on the cue at 0 if there is one, so edits go into it.
        if let Some(cue) = self.timeline.cues.iter().find(|c| c.time.abs() < 1e-3) {
            self.selected = Some(cue.id);
        }
    }

    pub(super) fn seek(&mut self, time: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.playhead = time.clamp(0.0, self.timeline.duration);
        self.sync_sliders(window, cx);
        self.push_values();
        cx.notify();
    }

    /// Select a cue and park the playhead on it (edit mode when stopped).
    pub(super) fn go_to_cue(&mut self, id: u32, window: &mut Window, cx: &mut Context<Self>) {
        let Some(time) = self.timeline.cue(id).map(|c| c.time) else {
            return;
        };
        self.selected = Some(id);
        self.seek(time, window, cx);
    }

    pub(super) fn step_cue(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let t = self.playhead;
        let target = if forward {
            self.timeline.cues.iter().find(|c| c.time > t + 1e-3)
        } else {
            self.timeline.cues.iter().rev().find(|c| c.time < t - 1e-3)
        };
        if let Some(cue) = target.map(|c| c.id) {
            self.go_to_cue(cue, window, cx);
        }
    }

    /// Snapshot everything (cues + live overrides) into a new cue at the
    /// playhead; the overrides are absorbed by it.
    pub(super) fn add_cue(&mut self, cx: &mut Context<Self>) {
        if let Some(existing) = self
            .timeline
            .cues
            .iter()
            .find(|c| (c.time - self.playhead).abs() < 1e-3)
        {
            let id = existing.id;
            self.selected = Some(id);
            self.status = format!("already on cue {id}; slider changes edit it");
            cx.notify();
            return;
        }
        let values = self.effective_values();
        let id = self
            .timeline
            .add_cue(self.playhead, Transition::Interpolate, values);
        self.overrides.clear();
        self.selected = Some(id);
        self.status = format!("added cue {id} at {:.2}s", self.playhead);
        self.push_values();
        cx.notify();
    }

    /// Bake current live values into the selected cue.
    pub(super) fn bake_into_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        let values = self.effective_values();
        if let Some(cue) = self.timeline.cue_mut(id) {
            cue.values = values;
            self.overrides.clear();
            self.status = format!("baked live values into cue {id}");
        }
        self.push_values();
        cx.notify();
    }

    pub(super) fn delete_selected(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        self.timeline.remove_cue(id);
        self.selected = self.timeline.cues.first().map(|c| c.id);
        self.status = format!("deleted cue {id}");
        cx.notify();
    }

    pub(super) fn toggle_transition(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.selected else { return };
        if let Some(cue) = self.timeline.cue_mut(id) {
            cue.transition = cue.transition.toggled();
        }
        cx.notify();
    }

    pub(super) fn adjust_duration(&mut self, delta: f32, cx: &mut Context<Self>) {
        self.timeline.duration = (self.timeline.duration + delta).clamp(1.0, 600.0);
        for cue in &mut self.timeline.cues {
            cue.time = cue.time.min(self.timeline.duration);
        }
        self.playhead = self.playhead.min(self.timeline.duration);
        cx.notify();
    }

    pub(super) fn save(&mut self, cx: &mut Context<Self>) {
        match self
            .timeline
            .to_json()
            .map_err(|e| e.to_string())
            .and_then(|json| std::fs::write(&self.file, json).map_err(|e| e.to_string()))
        {
            Ok(()) => self.status = format!("saved {}", self.file.display()),
            Err(e) => self.status = format!("save failed: {e}"),
        }
        cx.notify();
    }

    pub(super) fn load(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match std::fs::read_to_string(&self.file)
            .map_err(|e| e.to_string())
            .and_then(|json| Timeline::from_json(&json).map_err(|e| e.to_string()))
        {
            Ok(timeline) => {
                self.timeline = timeline;
                self.source_sliders.clear();
                self.ensure_source_sliders(cx);
                self.mod_dirty = true;
                self.selected = self.timeline.cues.first().map(|c| c.id);
                self.status = format!("loaded {}", self.file.display());
                self.seek(self.playhead, window, cx);
            }
            Err(e) => self.status = format!("load failed: {e}"),
        }
        cx.notify();
    }
}
