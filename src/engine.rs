use std::{collections::BTreeSet, sync::Arc};

use crate::{
    config::{Action, BindingSummary, CompiledConfig, FeedbackPolicy},
    gestures::{Scroll, SwipeRecognizer},
    key::{Chord, Key, Modifier},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Down,
    Up,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Input {
    Scroll(Scroll),
    Key {
        key: Key,
        kind: EventKind,
        repeat: bool,
    },
    Modifier {
        modifier: Modifier,
        kind: EventKind,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Decision {
    Pass,
    Suppress,
    PassWithModifiers(Vec<Modifier>),
    Trigger(Action),
}

pub struct Engine {
    config: CompiledConfig,
    held_modifiers: BTreeSet<Modifier>,
    hyper_active: bool,
    hyper_used: bool,
    consumed_keys: BTreeSet<Key>,
    rewritten_keys: BTreeSet<Key>,
    swipe: SwipeRecognizer,
    last_gesture: Option<Chord>,
}

impl Engine {
    pub fn new(config: CompiledConfig) -> Self {
        Self {
            config,
            held_modifiers: BTreeSet::new(),
            hyper_active: false,
            hyper_used: false,
            consumed_keys: BTreeSet::new(),
            rewritten_keys: BTreeSet::new(),
            swipe: SwipeRecognizer::default(),
            last_gesture: None,
        }
    }

    pub fn handle(&mut self, input: Input) -> Decision {
        self.last_gesture = None;
        match input {
            Input::Scroll(scroll) => self.handle_scroll(scroll),
            Input::Modifier { modifier, kind } => {
                match kind {
                    EventKind::Down => {
                        self.held_modifiers.insert(modifier);
                        if self.hyper_active {
                            self.hyper_used = true;
                        }
                    }
                    EventKind::Up => {
                        self.held_modifiers.remove(&modifier);
                    }
                }
                Decision::Pass
            }
            Input::Key { key, kind, repeat } => self.handle_key(key, kind, repeat),
        }
    }

    pub fn last_gesture(&self) -> Option<Chord> {
        self.last_gesture.clone()
    }

    pub fn hyper_active(&self) -> bool {
        self.hyper_active
    }

    fn gesture_chord(&self, name: &str) -> Chord {
        let mut modifiers: Vec<_> = self.held_modifiers.iter().copied().collect();
        modifiers.push(Modifier::Hyper);
        Chord::new(modifiers, name.parse().expect("known gesture"))
    }

    fn handle_scroll(&mut self, scroll: Scroll) -> Decision {
        let right = self
            .config
            .action_for(&self.gesture_chord("swipe_right"))
            .is_some();
        let left = self
            .config
            .action_for(&self.gesture_chord("swipe_left"))
            .is_some();
        let outcome = self.swipe.handle(scroll, self.hyper_active, left, right);
        if let Some(name) = outcome.gesture {
            self.hyper_used = true;
            let chord = self.gesture_chord(name);
            self.last_gesture = Some(chord.clone());
            if let Some(action) = self.config.action_for(&chord) {
                return Decision::Trigger(action.clone());
            }
        }
        if outcome.suppress {
            self.hyper_used = true;
            Decision::Suppress
        } else {
            Decision::Pass
        }
    }

    pub fn is_idle(&self) -> bool {
        self.held_modifiers.is_empty()
            && !self.hyper_active
            && self.consumed_keys.is_empty()
            && self.rewritten_keys.is_empty()
    }

    pub fn hyper_key(&self) -> &Key {
        &self.config.hyper.key
    }

    pub(crate) fn cheatsheet(&self) -> Option<(Arc<[BindingSummary]>, u64)> {
        self.config.ui.cheatsheet.then(|| {
            (
                self.config.hyper_binding_summary(),
                self.config.ui.cheatsheet_delay_ms,
            )
        })
    }

    pub(crate) fn feedback_policy(&self) -> FeedbackPolicy {
        self.config.ui.feedback
    }

    pub fn replace_config(&mut self, config: CompiledConfig) {
        self.config = config;
    }

    pub fn preview_chord(&self, input: &Input) -> Option<Chord> {
        let Input::Key { key, kind, repeat } = input else {
            return None;
        };
        if *kind != EventKind::Down || *repeat || key == self.hyper_key() {
            return None;
        }

        let mut modifiers: Vec<_> = self.held_modifiers.iter().copied().collect();
        if self.hyper_active {
            modifiers.push(Modifier::Hyper);
        }
        Some(Chord::new(modifiers, key.clone()))
    }

    fn handle_key(&mut self, key: Key, kind: EventKind, repeat: bool) -> Decision {
        if key == self.config.hyper.key {
            return self.handle_hyper_key(kind);
        }

        if kind == EventKind::Up && self.consumed_keys.remove(&key) {
            return Decision::Suppress;
        }
        if kind == EventKind::Up && self.rewritten_keys.remove(&key) {
            return Decision::PassWithModifiers(self.config.hyper.modifiers.clone());
        }
        if repeat && self.consumed_keys.contains(&key) {
            return Decision::Suppress;
        }

        if kind == EventKind::Down {
            if self.hyper_active {
                self.hyper_used = true;
            }
            let mut modifiers: Vec<_> = self.held_modifiers.iter().copied().collect();
            if self.hyper_active {
                modifiers.push(Modifier::Hyper);
            }
            let chord = Chord::new(modifiers, key.clone());
            if let Some(action) = self.config.action_for(&chord).cloned() {
                self.consumed_keys.insert(key);
                return Decision::Trigger(action);
            }
            if self.hyper_active {
                self.rewritten_keys.insert(key);
                return Decision::PassWithModifiers(self.config.hyper.modifiers.clone());
            }
        }

        Decision::Pass
    }

    fn handle_hyper_key(&mut self, kind: EventKind) -> Decision {
        match kind {
            EventKind::Down => {
                self.hyper_active = true;
                self.hyper_used = false;
                Decision::Suppress
            }
            EventKind::Up => {
                self.swipe.release_hyper();
                self.hyper_active = false;
                if self.hyper_used {
                    Decision::Suppress
                } else {
                    Decision::Trigger(Action::SendKeys(self.config.hyper.tap.clone()))
                }
            }
        }
    }
}
