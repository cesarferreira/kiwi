use std::collections::BTreeSet;

use crate::{
    config::{Action, CompiledConfig},
    key::{Chord, Key, Modifier},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EventKind {
    Down,
    Up,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Input {
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
        }
    }

    pub fn handle(&mut self, input: Input) -> Decision {
        match input {
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
