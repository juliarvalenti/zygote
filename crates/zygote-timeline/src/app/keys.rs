//! Learned keys: binding a key to a cue or an envelope trigger, and firing them.

use super::*;

impl TimelineApp {
    pub(super) fn key_for_action(&self, wanted: &KeyAction) -> Option<&str> {
        self.timeline
            .keys
            .iter()
            .find(|(_, a)| *a == wanted)
            .map(|(k, _)| k.as_str())
    }

    pub(super) fn key_down(
        &mut self,
        ev: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let name = ev.keystroke.unparse();
        if let Some(target) = self.learning.take() {
            if RESERVED_KEYS.contains(&name.as_str()) {
                self.status = format!("`{name}` is a transport key; pick another");
                self.learning = Some(target);
            } else {
                let action = match &target {
                    LearnTarget::Trigger(t) => KeyAction::Trigger { trigger: t.clone() },
                    LearnTarget::Cue(id) => KeyAction::Cue { id: *id },
                };
                self.timeline.keys.retain(|_, a| a != &action);
                self.timeline.keys.insert(name.clone(), action);
                self.status = format!("bound `{name}`");
            }
            cx.notify();
            return;
        }
        if ev.is_held || self.held_keys.contains(&name) {
            return;
        }
        let Some(action) = self.timeline.keys.get(&name).cloned() else {
            return;
        };
        self.held_keys.insert(name);
        match action {
            KeyAction::Trigger { trigger } => self.gate(&trigger, true, cx),
            KeyAction::Cue { id } => self.go_to_cue(id, window, cx),
        }
    }

    pub(super) fn key_up(&mut self, ev: &KeyUpEvent, cx: &mut Context<Self>) {
        let name = ev.keystroke.unparse();
        if !self.held_keys.remove(&name) {
            return;
        }
        if let Some(KeyAction::Trigger { trigger }) = self.timeline.keys.get(&name).cloned() {
            self.gate(&trigger, false, cx);
        }
    }
}
