use kiwi_keymapper::{
    config::{Action, Config},
    engine::{Decision, Engine, EventKind, Input},
    key::{Key, Modifier},
};

fn key(name: &str) -> Key {
    name.parse().unwrap()
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
        Decision::Trigger(Action::LaunchApp("Ghostty".into()))
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
        Decision::Trigger(Action::LaunchApp("Specific".into()))
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
