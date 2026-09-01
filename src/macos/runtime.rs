use std::sync::mpsc::Receiver;

use crate::{
    config::{Action, CompiledConfig},
    engine::{Decision, Engine, Input},
    key::{Chord, Key},
};

use super::feedback::ActionJob;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum ReloadNotice {
    Applied(usize),
    HyperKeyChanged,
}

pub(crate) struct HandledEvent {
    pub decision: Decision,
    pub chord: Option<Chord>,
    pub action_job: Option<ActionJob>,
    pub notices: Vec<ReloadNotice>,
}

fn job_chord(action: &Action, triggering: Option<Chord>, pressed: Option<Key>) -> Option<Chord> {
    if let Some(chord) = triggering {
        return Some(chord);
    }
    // Hyper and dual-role taps resolve on key-up, so the physical key alone
    // does not describe what the user asked for.
    if let Action::SendKeys(emitted) = action {
        return Some(emitted.clone());
    }
    pressed.map(|key| Chord::new(Vec::new(), key))
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

        let preview_chord = self.engine.preview_chord(&input);
        let chord = preview.then(|| preview_chord.clone()).flatten();
        let pressed_key = match &input {
            Input::Key { key, .. } => Some(key.clone()),
            Input::Modifier { .. } => None,
        };
        let decision = self.engine.handle(input);
        let action_job = match &decision {
            Decision::Trigger(action) => {
                job_chord(action, preview_chord, pressed_key).map(|chord| ActionJob {
                    chord,
                    action: action.clone(),
                    feedback: self.engine.feedback_policy(),
                })
            }
            _ => None,
        };

        self.apply_if_idle(&mut notices);
        HandledEvent {
            decision,
            chord,
            action_job,
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
        config::{Action, AppAction, AppBehavior, Config, FeedbackPolicy},
        engine::{Decision, EventKind, Input},
        key::Key,
    };

    fn config(contents: &str) -> crate::config::CompiledConfig {
        Config::from_toml(contents).unwrap().compile().unwrap()
    }

    fn launch(target: &str) -> Action {
        Action::App(AppAction {
            target: target.into(),
            behavior: AppBehavior::Launch,
        })
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
        assert_eq!(old_binding.decision, Decision::Trigger(launch("Old")));
        assert!(old_binding.notices.is_empty());

        runtime.handle(release("a"));
        let boundary = runtime.handle(release("caps_lock"));
        assert_eq!(boundary.notices, vec![ReloadNotice::Applied(1)]);

        runtime.handle(press("caps_lock"));
        assert_eq!(
            runtime.handle(press("b")).decision,
            Decision::Trigger(launch("New"))
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
            Decision::Trigger(launch("Old"))
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

    #[test]
    fn lost_passthrough_key_up_does_not_block_reload() {
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

        assert_eq!(runtime.handle(press("q")).decision, Decision::Pass);
        sender.send(replacement).unwrap();
        let boundary = runtime.handle(press("caps_lock"));
        assert_eq!(boundary.notices, vec![ReloadNotice::Applied(1)]);
        assert_eq!(
            runtime.handle(press("b")).decision,
            Decision::Trigger(launch("New"))
        );
    }

    #[test]
    fn action_job_retains_normalized_chord_action_and_active_feedback_generation() {
        let initial = config(
            r#"
            [ui]
            feedback = "errors"

            [bindings]
            "Shift+Hyper+A" = { app = "Old" }
            "#,
        );
        let replacement = config(
            r#"
            [ui]
            feedback = "all"

            [bindings]
            "hyper+b" = { app = "New" }
            "#,
        );
        let (sender, receiver) = mpsc::channel();
        let mut runtime = ReloadingEngine::new(initial, receiver);

        runtime.handle(press("caps_lock"));
        runtime.handle(Input::Modifier {
            modifier: "shift".parse().unwrap(),
            kind: EventKind::Down,
        });
        sender.send(replacement).unwrap();
        let old = runtime.handle(press("a")).action_job.unwrap();
        assert_eq!(old.chord.to_string(), "hyper+shift+a");
        assert_eq!(old.action, launch("Old"));
        assert_eq!(old.feedback, FeedbackPolicy::Errors);

        runtime.handle(release("a"));
        runtime.handle(Input::Modifier {
            modifier: "shift".parse().unwrap(),
            kind: EventKind::Up,
        });
        runtime.handle(release("caps_lock"));

        runtime.handle(press("caps_lock"));
        let new = runtime.handle(press("b")).action_job.unwrap();
        assert_eq!(new.chord.to_string(), "hyper+b");
        assert_eq!(new.action, launch("New"));
        assert_eq!(new.feedback, FeedbackPolicy::All);
    }

    #[test]
    fn hyper_tap_job_reports_the_emitted_key_not_the_physical_hyper_key() {
        let initial = config(
            r#"
            [ui]
            feedback = "all"

            [hyper]
            key = "caps_lock"
            tap = "escape"
            "#,
        );
        let (_sender, receiver) = mpsc::channel();
        let mut runtime = ReloadingEngine::new(initial, receiver);

        runtime.handle(press("caps_lock"));
        let tap = runtime.handle(release("caps_lock")).action_job.unwrap();

        assert_eq!(tap.chord.to_string(), "escape");
        assert_eq!(tap.action, Action::SendKeys("escape".parse().unwrap()));
    }
}
