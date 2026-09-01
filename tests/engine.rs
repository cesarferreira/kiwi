use kiwi_keymapper::{
    config::{Action, AppAction, AppBehavior, Config},
    engine::{Decision, Engine, EventKind, Input},
    key::{Key, Modifier},
};

fn key(name: &str) -> Key {
    name.parse().unwrap()
}

fn launch(target: &str) -> Action {
    Action::App(AppAction {
        target: target.into(),
        behavior: AppBehavior::Launch,
    })
}

fn engine() -> Engine {
    let config = Config::from_toml(
        r#"
        [bindings]
        "hyper+t" = { app = "Ghostty" }
        "hyper+a" = { keys = "control+a" }
        "left_option+h" = { keys = "left" }
        "#,
    )
    .unwrap()
    .compile()
    .unwrap();
    Engine::new(config)
}

fn press(name: &str) -> Input {
    Input::Key {
        key: key(name),
        kind: EventKind::Down,
        repeat: false,
    }
}

fn release(name: &str) -> Input {
    Input::Key {
        key: key(name),
        kind: EventKind::Up,
        repeat: false,
    }
}

#[test]
fn tapping_caps_emits_escape_and_suppresses_caps() {
    let mut engine = engine();

    assert_eq!(engine.handle(press("caps_lock")), Decision::Suppress);
    assert_eq!(
        engine.handle(release("caps_lock")),
        Decision::Trigger(Action::SendKeys("escape".parse().unwrap()))
    );
}

#[test]
fn hyper_app_binding_triggers_once_and_consumes_key_pair() {
    let mut engine = engine();

    assert_eq!(engine.handle(press("caps_lock")), Decision::Suppress);
    assert_eq!(
        engine.handle(press("t")),
        Decision::Trigger(launch("Ghostty"))
    );
    assert_eq!(
        engine.handle(Input::Key {
            key: key("t"),
            kind: EventKind::Down,
            repeat: true,
        }),
        Decision::Suppress
    );
    assert_eq!(engine.handle(release("t")), Decision::Suppress);
    assert_eq!(engine.handle(release("caps_lock")), Decision::Suppress);
}

#[test]
fn unmapped_hyper_chord_passes_with_configured_modifiers() {
    let mut engine = engine();
    engine.handle(press("caps_lock"));

    assert_eq!(
        engine.handle(press("b")),
        Decision::PassWithModifiers(vec![
            Modifier::Command,
            Modifier::Control,
            Modifier::Option,
            Modifier::Shift,
        ])
    );
    assert!(matches!(
        engine.handle(release("b")),
        Decision::PassWithModifiers(_)
    ));
    assert_eq!(engine.handle(release("caps_lock")), Decision::Suppress);
}

#[test]
fn side_specific_modifier_binding_works_without_hyper() {
    let mut engine = engine();

    assert_eq!(
        engine.handle(Input::Modifier {
            modifier: Modifier::LeftOption,
            kind: EventKind::Down,
        }),
        Decision::Pass
    );
    assert_eq!(
        engine.handle(press("h")),
        Decision::Trigger(Action::SendKeys("left".parse().unwrap()))
    );
    assert_eq!(engine.handle(release("h")), Decision::Suppress);
}

#[test]
fn side_specific_binding_wins_over_a_matching_generic_binding() {
    let config = Config::from_toml(
        r#"
        [bindings]
        "option+h" = { app = "Generic" }
        "left_option+h" = { app = "Specific" }
        "#,
    )
    .unwrap()
    .compile()
    .unwrap();
    let mut engine = Engine::new(config);
    engine.handle(Input::Modifier {
        modifier: Modifier::LeftOption,
        kind: EventKind::Down,
    });

    assert_eq!(
        engine.handle(press("h")),
        Decision::Trigger(launch("Specific"))
    );
}

#[test]
fn pressing_another_key_prevents_escape_even_if_it_is_unmapped() {
    let mut engine = engine();
    engine.handle(press("caps_lock"));
    engine.handle(press("b"));
    engine.handle(release("b"));

    assert_eq!(engine.handle(release("caps_lock")), Decision::Suppress);
}

#[test]
fn engine_is_idle_only_between_complete_input_sequences() {
    let mut engine = engine();

    assert!(engine.is_idle());
    engine.handle(press("caps_lock"));
    assert!(!engine.is_idle());
    engine.handle(press("t"));
    engine.handle(release("t"));
    engine.handle(release("caps_lock"));
    assert!(engine.is_idle());
}

#[test]
fn replacing_config_changes_the_next_binding() {
    let replacement = Config::from_toml(
        r#"
        [bindings]
        "hyper+b" = { app = "Replacement" }
        "#,
    )
    .unwrap()
    .compile()
    .unwrap();
    let mut engine = engine();

    engine.replace_config(replacement);
    engine.handle(press("caps_lock"));
    assert_eq!(
        engine.handle(press("b")),
        Decision::Trigger(launch("Replacement"))
    );
}

#[test]
fn preview_chord_uses_held_physical_modifiers_and_hyper() {
    let mut engine = engine();
    engine.handle(Input::Modifier {
        modifier: Modifier::LeftOption,
        kind: EventKind::Down,
    });

    assert_eq!(
        engine.preview_chord(&press("h")).unwrap().to_string(),
        "left_option+h"
    );

    engine.handle(Input::Modifier {
        modifier: Modifier::LeftOption,
        kind: EventKind::Up,
    });
    engine.handle(press("caps_lock"));
    assert_eq!(
        engine.preview_chord(&press("h")).unwrap().to_string(),
        "hyper+h"
    );
}

#[test]
fn preview_chord_ignores_non_observable_inputs() {
    let engine = engine();

    assert_eq!(engine.preview_chord(&release("h")), None);
    assert_eq!(
        engine.preview_chord(&Input::Key {
            key: key("h"),
            kind: EventKind::Down,
            repeat: true,
        }),
        None
    );
    assert_eq!(engine.preview_chord(&press("caps_lock")), None);
    assert_eq!(
        engine.preview_chord(&Input::Modifier {
            modifier: Modifier::LeftOption,
            kind: EventKind::Down,
        }),
        None
    );
}
