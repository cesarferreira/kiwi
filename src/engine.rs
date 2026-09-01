use std::collections::{BTreeMap, BTreeSet};

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

struct ActiveDualRole {
    used: bool,
    tap_modifiers: Vec<Modifier>,
}

pub struct Engine {
    config: CompiledConfig,
    held_modifiers: BTreeSet<Modifier>,
    hyper_active: bool,
    hyper_used: bool,
    active_dual_roles: BTreeMap<Key, ActiveDualRole>,
    released_dual_roles: BTreeSet<Key>,
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
            active_dual_roles: BTreeMap::new(),
            released_dual_roles: BTreeSet::new(),
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
                        self.mark_dual_roles_used();
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

    pub fn is_idle(&self) -> bool {
        self.held_modifiers.is_empty()
            && !self.hyper_active
            && self.active_dual_roles.is_empty()
            && self.consumed_keys.is_empty()
            && self.rewritten_keys.is_empty()
    }

    pub fn hyper_key(&self) -> &Key {
        &self.config.hyper.key
    }

    pub fn replace_config(&mut self, config: CompiledConfig) {
        self.config = config;
        self.released_dual_roles.clear();
    }

    pub fn preview_chord(&self, input: &Input) -> Option<Chord> {
        let Input::Key { key, kind, repeat } = input else {
            return None;
        };
        if *kind != EventKind::Down
            || *repeat
            || key == self.hyper_key()
            || self.config.dual_roles.iter().any(|role| &role.key == key)
        {
            return None;
        }

        Some(self.current_chord(key.clone()))
    }

    fn handle_key(&mut self, key: Key, kind: EventKind, repeat: bool) -> Decision {
        if key == self.config.hyper.key {
            return self.handle_hyper_key(kind, repeat);
        }
        if let Some(role) = self
            .config
            .dual_roles
            .iter()
            .find(|role| role.key == key)
            .cloned()
        {
            return self.handle_dual_role_key(role.key, role.tap, kind, repeat);
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
            self.mark_dual_roles_used();
            let chord = self.current_chord(key.clone());
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

    fn handle_hyper_key(&mut self, kind: EventKind, repeat: bool) -> Decision {
        match kind {
            EventKind::Down => {
                if repeat || self.hyper_active {
                    return Decision::Suppress;
                }
                self.hyper_active = true;
                self.hyper_used = false;
                self.mark_dual_roles_used();
                Decision::Suppress
            }
            EventKind::Up => {
                if !self.hyper_active {
                    return Decision::Suppress;
                }
                self.hyper_active = false;
                if self.hyper_used {
                    Decision::Suppress
                } else {
                    Decision::Trigger(Action::SendKeys(self.config.hyper.tap.clone()))
                }
            }
        }
    }

    fn handle_dual_role_key(
        &mut self,
        key: Key,
        tap: Chord,
        kind: EventKind,
        repeat: bool,
    ) -> Decision {
        match kind {
            EventKind::Down => {
                if repeat || self.active_dual_roles.contains_key(&key) {
                    return Decision::Suppress;
                }
                if self.hyper_active {
                    self.hyper_used = true;
                }
                self.mark_dual_roles_used();
                self.released_dual_roles.remove(&key);
                self.active_dual_roles.insert(
                    key,
                    ActiveDualRole {
                        used: false,
                        tap_modifiers: self.held_modifiers.iter().cloned().collect(),
                    },
                );
                Decision::Suppress
            }
            EventKind::Up => {
                let Some(active) = self.active_dual_roles.remove(&key) else {
                    return if self.released_dual_roles.contains(&key) {
                        Decision::Suppress
                    } else {
                        Decision::Pass
                    };
                };
                self.released_dual_roles.insert(key);
                if active.used {
                    Decision::Suppress
                } else {
                    let mut modifiers = tap.modifiers;
                    modifiers.extend(active.tap_modifiers);
                    Decision::Trigger(Action::SendKeys(Chord::new(modifiers, tap.key)))
                }
            }
        }
    }

    fn mark_dual_roles_used(&mut self) {
        for active in self.active_dual_roles.values_mut() {
            active.used = true;
        }
    }

    fn current_chord(&self, key: Key) -> Chord {
        let mut modifiers: Vec<_> = self.held_modifiers.iter().cloned().collect();
        if self.hyper_active {
            modifiers.push(Modifier::Hyper);
        }
        modifiers.extend(self.active_dual_roles.keys().filter_map(|active_key| {
            self.config
                .dual_roles
                .iter()
                .find(|role| &role.key == active_key)
                .map(|role| Modifier::Named(role.hold_modifier.clone()))
        }));
        Chord::new(modifiers, key)
    }
}
