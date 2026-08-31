use std::sync::mpsc::Receiver;

use crate::{
    config::CompiledConfig,
    engine::{Decision, Engine, Input},
    key::Chord,
};

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ReloadNotice {
    Applied(usize),
    HyperKeyChanged,
}

pub(crate) struct HandledEvent {
    pub decision: Decision,
    pub chord: Option<Chord>,
    pub notices: Vec<ReloadNotice>,
}

pub(crate) struct ReloadingEngine {
    engine: Engine,
    config_receiver: Receiver<CompiledConfig>,
    pending_config: Option<CompiledConfig>,
}

impl ReloadingEngine {
    pub fn new(config: CompiledConfig, config_receiver: Receiver<CompiledConfig>) -> Self {
        Self {
            engine: Engine::new(config),
            config_receiver,
            pending_config: None,
        }
    }

    pub fn handle(&mut self, input: Input) -> HandledEvent {
        self.process(input, false)
    }

    pub fn observe(&mut self, input: Input) -> HandledEvent {
        self.process(input, true)
    }

    fn process(&mut self, input: Input, preview: bool) -> HandledEvent {
        let mut notices = Vec::new();
        self.drain_configs(&mut notices);
        self.apply_if_idle(&mut notices);

        let chord = preview.then(|| self.engine.preview_chord(&input)).flatten();
        let decision = self.engine.handle(input);

        self.apply_if_idle(&mut notices);
        HandledEvent {
            decision,
            chord,
            notices,
        }
    }

    fn drain_configs(&mut self, notices: &mut Vec<ReloadNotice>) {
        let mut hyper_key_changed = false;
        for config in self.config_receiver.try_iter() {
            if &config.hyper.key == self.engine.hyper_key() {
                self.pending_config = Some(config);
            } else {
                hyper_key_changed = true;
            }
        }
        if hyper_key_changed {
            notices.push(ReloadNotice::HyperKeyChanged);
        }
    }

    fn apply_if_idle(&mut self, notices: &mut Vec<ReloadNotice>) {
        if !self.engine.is_idle() {
            return;
        }
        if let Some(config) = self.pending_config.take() {
            let binding_count = config.bindings.len();
            self.engine.replace_config(config);
            notices.push(ReloadNotice::Applied(binding_count));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{ReloadNotice, ReloadingEngine};
    use crate::{
        config::{Action, Config},
        engine::{Decision, EventKind, Input},
        key::Key,
    };

    fn config(contents: &str) -> crate::config::CompiledConfig {
        Config::from_toml(contents).unwrap().compile().unwrap()
    }

    fn press(name: &str) -> Input {
        Input::Key {
            key: name.parse::<Key>().unwrap(),
            kind: EventKind::Down,
            repeat: false,
        }
    }

    fn release(name: &str) -> Input {
        Input::Key {
            key: name.parse::<Key>().unwrap(),
            kind: EventKind::Up,
            repeat: false,
        }
    }

    #[test]
    fn pending_config_applies_only_after_the_active_chord_finishes() {
        let initial = config(
            r#"
            [bindings]
            "hyper+a" = { app = "Old" }
            "#,
        );
        let replacement = config(
            r#"
            [bindings]
            "hyper+b" = { app = "New" }
            "#,
        );
        let (sender, receiver) = mpsc::channel();
        let mut runtime = ReloadingEngine::new(initial, receiver);

        runtime.handle(press("caps_lock"));
        sender.send(replacement).unwrap();
        let old_binding = runtime.handle(press("a"));
        assert_eq!(
            old_binding.decision,
            Decision::Trigger(Action::LaunchApp("Old".into()))
        );
        assert!(old_binding.notices.is_empty());

        runtime.handle(release("a"));
        let boundary = runtime.handle(release("caps_lock"));
        assert_eq!(boundary.notices, vec![ReloadNotice::Applied(1)]);

        runtime.handle(press("caps_lock"));
        assert_eq!(
            runtime.handle(press("b")).decision,
            Decision::Trigger(Action::LaunchApp("New".into()))
        );
    }

    #[test]
    fn hyper_key_change_is_rejected_without_replacing_bindings() {
        let initial = config(
            r#"
            [bindings]
            "hyper+a" = { app = "Old" }
            "#,
        );
        let incompatible = config(
            r#"
            [hyper]
            key = "f18"

            [bindings]
            "hyper+b" = { app = "New" }
            "#,
        );
        let (sender, receiver) = mpsc::channel();
        let mut runtime = ReloadingEngine::new(initial, receiver);
        sender.send(incompatible).unwrap();

        let hyper_down = runtime.handle(press("caps_lock"));
        assert_eq!(hyper_down.notices, vec![ReloadNotice::HyperKeyChanged]);
        assert_eq!(hyper_down.decision, Decision::Suppress);
        assert_eq!(
            runtime.handle(press("a")).decision,
            Decision::Trigger(Action::LaunchApp("Old".into()))
        );
    }

    #[test]
    fn observe_previews_a_chord_but_handle_does_not() {
        let initial = config(
            r#"
            [bindings]
            "hyper+a" = { app = "Observed" }
            "#,
        );
        let (_sender, receiver) = mpsc::channel();
        let mut runtime = ReloadingEngine::new(initial, receiver);

        runtime.observe(press("caps_lock"));
        let observed = runtime.observe(press("a"));
        assert_eq!(observed.chord.unwrap().to_string(), "hyper+a");

        runtime.observe(release("a"));
        runtime.observe(release("caps_lock"));
        runtime.handle(press("caps_lock"));
        assert!(runtime.handle(press("a")).chord.is_none());
    }
}
