use std::{
    collections::BTreeSet,
    sync::Arc,
    time::{Duration, Instant},
};

use crate::{
    config::{Action, BindingSummary, CompiledConfig, FeedbackPolicy},
    gestures::{GestureBindings, Scroll, SwipeRecognizer},
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
    /// Run the action but deliver the event, so a modifier release still
    /// reaches apps.
    PassAndTrigger(Action),
}

/// Longest press that still counts as a tap.
pub const DOUBLE_TAP_HOLD: Duration = Duration::from_millis(250);
/// Longest pause between the first release and the second press.
pub const DOUBLE_TAP_GAP: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, Debug)]
struct ModifierTap {
    modifier: Modifier,
    pressed_at: Duration,
    released_at: Option<Duration>,
    second: bool,
}

pub struct Engine {
    config: CompiledConfig,
    held_modifiers: BTreeSet<Modifier>,
    hyper_active: bool,
    hyper_used: bool,
    consumed_keys: BTreeSet<Key>,
    rewritten_keys: BTreeSet<Key>,
    swipe: SwipeRecognizer,
    last_virtual_chord: Option<Chord>,
    cheatsheet_requested: bool,
    modifier_tap: Option<ModifierTap>,
    started: Instant,
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
            last_virtual_chord: None,
            cheatsheet_requested: false,
            modifier_tap: None,
            started: Instant::now(),
        }
    }

    pub fn handle(&mut self, input: Input) -> Decision {
        self.handle_at(input, self.started.elapsed())
    }

    /// Handles `input` as if it arrived `now` after an arbitrary fixed origin.
    pub fn handle_at(&mut self, input: Input, now: Duration) -> Decision {
        self.last_virtual_chord = None;
        self.cheatsheet_requested = false;
        if !matches!(input, Input::Modifier { .. }) {
            self.modifier_tap = None;
        }
        match input {
            Input::Scroll(scroll) => self.handle_scroll(scroll),
            Input::Modifier { modifier, kind } => {
                let decision = self.handle_modifier_tap(modifier, kind, now);
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
                decision
            }
            Input::Key { key, kind, repeat } => self.handle_key(key, kind, repeat),
        }
    }

    /// The swipe or double-tap chord matched by the last input, which has no
    /// physical key of its own.
    pub fn last_virtual_chord(&self) -> Option<Chord> {
        self.last_virtual_chord.clone()
    }

    /// True when the last handled input was an unbound Hyper+`/` (the `?` key)
    /// asking for the cheatsheet right away.
    pub fn cheatsheet_requested(&self) -> bool {
        self.cheatsheet_requested
    }

    pub fn hyper_active(&self) -> bool {
        self.hyper_active
    }

    fn gesture_chord(&self, name: &str) -> Chord {
        let mut modifiers: Vec<_> = self.held_modifiers.iter().copied().collect();
        modifiers.push(Modifier::Hyper);
        Chord::new(modifiers, name.parse().expect("known gesture"))
    }

    // Called before `held_modifiers` records this event. The modifier events
    // always pass through, so ordinary shortcuts and typing never wait on it.
    fn handle_modifier_tap(
        &mut self,
        modifier: Modifier,
        kind: EventKind,
        now: Duration,
    ) -> Decision {
        let alone = !self.hyper_active && self.held_modifiers.iter().all(|held| *held == modifier);
        if !alone {
            self.modifier_tap = None;
            return Decision::Pass;
        }
        match kind {
            EventKind::Down => {
                let second = self.modifier_tap.is_some_and(|tap| {
                    tap.modifier == modifier
                        && !tap.second
                        && tap
                            .released_at
                            .is_some_and(|released| now.saturating_sub(released) <= DOUBLE_TAP_GAP)
                });
                self.modifier_tap = Some(ModifierTap {
                    modifier,
                    pressed_at: now,
                    released_at: None,
                    second,
                });
                Decision::Pass
            }
            EventKind::Up => {
                let Some(tap) = self.modifier_tap.take() else {
                    return Decision::Pass;
                };
                if tap.modifier != modifier
                    || tap.released_at.is_some()
                    || now.saturating_sub(tap.pressed_at) > DOUBLE_TAP_HOLD
                {
                    return Decision::Pass;
                }
                if !tap.second {
                    self.modifier_tap = Some(ModifierTap {
                        released_at: Some(now),
                        ..tap
                    });
                    return Decision::Pass;
                }
                let chord = Chord::new(vec![modifier], Key::double_tap());
                match self.config.action_for(&chord).cloned() {
                    Some(action) => {
                        self.last_virtual_chord = Some(chord);
                        Decision::PassAndTrigger(action)
                    }
                    None => Decision::Pass,
                }
            }
        }
    }

    fn handle_scroll(&mut self, scroll: Scroll) -> Decision {
        let bindings = GestureBindings {
            left: self
                .config
                .action_for(&self.gesture_chord("swipe_left"))
                .is_some(),
            right: self
                .config
                .action_for(&self.gesture_chord("swipe_right"))
                .is_some(),
            up: self
                .config
                .action_for(&self.gesture_chord("swipe_up"))
                .is_some(),
            down: self
                .config
                .action_for(&self.gesture_chord("swipe_down"))
                .is_some(),
        };
        let outcome = self.swipe.handle(scroll, self.hyper_active, bindings);
        if let Some(name) = outcome.gesture {
            self.hyper_used = true;
            let chord = self.gesture_chord(name);
            self.last_virtual_chord = Some(chord.clone());
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

    pub(crate) fn cheatsheet_rows(&self) -> Arc<[BindingSummary]> {
        self.config.hyper_binding_summary()
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
            if self.hyper_active && key.as_str() == "slash" {
                self.consumed_keys.insert(key);
                self.cheatsheet_requested = true;
                return Decision::Suppress;
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
