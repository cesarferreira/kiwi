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

fn dual_role_engine() -> Engine {
    let config = Config::from_toml(
        r#"
        [[dual_role]]
        key = "space"
        tap = "space"
        hold_modifier = "leader"

        [bindings]
        "leader+f" = { app = "Finder" }
        "hyper+leader+t" = { app = "Terminal" }
        "#,
    )
    .unwrap()
    .compile()
    .unwrap();
    Engine::new(config)
}

fn nested_dual_role_engine() -> Engine {
    let config = Config::from_toml(
        r#"
        [[dual_role]]
        key = "space"
        tap = "space"
        hold_modifier = "leader"

        [[dual_role]]
        key = "tab"
        tap = "tab"
        hold_modifier = "nav"

        [bindings]
        "leader+nav+f" = { app = "Both" }
        "nav+f" = { app = "Nav" }
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
        Decision::Trigger(Action::LaunchApp("Replacement".into()))
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

#[test]
fn dual_role_tap_emits_once_and_suppresses_repeats() {
    let mut engine = dual_role_engine();

    assert_eq!(engine.handle(press("space")), Decision::Suppress);
    assert_eq!(
        engine.handle(Input::Key {
            key: key("space"),
            kind: EventKind::Down,
            repeat: true,
        }),
        Decision::Suppress
    );
    assert_eq!(
        engine.handle(release("space")),
        Decision::Trigger(Action::SendKeys("space".parse().unwrap()))
    );
    assert_eq!(engine.handle(release("space")), Decision::Suppress);
}

#[test]
fn dual_role_hold_triggers_binding_without_tap() {
    let mut engine = dual_role_engine();

    assert_eq!(engine.handle(press("space")), Decision::Suppress);
    assert_eq!(
        engine.handle(press("f")),
        Decision::Trigger(Action::LaunchApp("Finder".into()))
    );
    assert_eq!(
        engine.handle(Input::Key {
            key: key("f"),
            kind: EventKind::Down,
            repeat: true,
        }),
        Decision::Suppress
    );
    assert_eq!(engine.handle(release("f")), Decision::Suppress);
    assert_eq!(engine.handle(release("space")), Decision::Suppress);
}

#[test]
fn unmapped_named_hold_layer_passes_secondary_key_unchanged() {
    let mut engine = dual_role_engine();

    assert_eq!(engine.handle(press("space")), Decision::Suppress);
    assert_eq!(engine.handle(press("b")), Decision::Pass);
    assert!(!engine.is_idle());
    assert_eq!(engine.handle(release("space")), Decision::Suppress);
    assert!(engine.is_idle());
    assert_eq!(engine.handle(release("b")), Decision::Pass);
    assert!(engine.is_idle());
}

#[test]
fn nested_hyper_and_dual_role_match_both_virtual_modifiers() {
    let mut engine = dual_role_engine();

    assert_eq!(engine.handle(press("caps_lock")), Decision::Suppress);
    assert_eq!(engine.handle(press("space")), Decision::Suppress);
    assert_eq!(
        engine.handle(press("t")),
        Decision::Trigger(Action::LaunchApp("Terminal".into()))
    );
    assert_eq!(engine.handle(release("t")), Decision::Suppress);
    assert_eq!(engine.handle(release("space")), Decision::Suppress);
    assert_eq!(engine.handle(release("caps_lock")), Decision::Suppress);
    assert!(engine.is_idle());
}

#[test]
fn dual_role_preview_contains_named_and_nested_virtual_modifiers() {
    let mut engine = dual_role_engine();

    engine.handle(press("space"));
    assert_eq!(
        engine.preview_chord(&press("f")).unwrap().to_string(),
        "leader+f"
    );
    engine.handle(press("caps_lock"));
    assert_eq!(
        engine.preview_chord(&press("t")).unwrap().to_string(),
        "hyper+leader+t"
    );
}

#[test]
fn replacing_config_changes_dual_roles_at_an_idle_boundary() {
    let replacement = Config::from_toml(
        r#"
        [[dual_role]]
        key = "tab"
        tap = "escape"
        hold_modifier = "nav"

        [bindings]
        "nav+h" = { app = "Replacement" }
        "#,
    )
    .unwrap()
    .compile()
    .unwrap();
    let mut engine = dual_role_engine();

    engine.replace_config(replacement);
    assert_eq!(engine.handle(press("space")), Decision::Pass);
    assert_eq!(engine.handle(release("space")), Decision::Pass);
    assert_eq!(engine.handle(press("tab")), Decision::Suppress);
    assert_eq!(
        engine.handle(press("h")),
        Decision::Trigger(Action::LaunchApp("Replacement".into()))
    );
}

#[test]
fn physical_modifiers_held_before_dual_role_down_compose_into_tap() {
    for modifier in [Modifier::Command, Modifier::Shift] {
        let mut engine = dual_role_engine();
        assert_eq!(
            engine.handle(Input::Modifier {
                modifier: modifier.clone(),
                kind: EventKind::Down,
            }),
            Decision::Pass
        );
        assert_eq!(engine.handle(press("space")), Decision::Suppress);
        assert_eq!(
            engine.handle(release("space")),
            Decision::Trigger(Action::SendKeys(kiwi_keymapper::key::Chord::new(
                vec![modifier],
                key("space"),
            )))
        );
    }
}

#[test]
fn physical_modifier_pressed_after_dual_role_down_suppresses_tap() {
    let mut engine = dual_role_engine();

    engine.handle(press("space"));
    engine.handle(Input::Modifier {
        modifier: Modifier::Shift,
        kind: EventKind::Down,
    });
    engine.handle(Input::Modifier {
        modifier: Modifier::Shift,
        kind: EventKind::Up,
    });

    assert_eq!(engine.handle(release("space")), Decision::Suppress);
}

#[test]
fn two_active_dual_roles_match_both_named_modifiers() {
    let mut engine = nested_dual_role_engine();

    engine.handle(press("space"));
    engine.handle(press("tab"));
    assert_eq!(
        engine.preview_chord(&press("f")).unwrap().to_string(),
        "leader+nav+f"
    );
    assert_eq!(
        engine.handle(press("f")),
        Decision::Trigger(Action::LaunchApp("Both".into()))
    );
    assert_eq!(engine.handle(release("f")), Decision::Suppress);
    assert_eq!(engine.handle(release("tab")), Decision::Suppress);
    assert_eq!(engine.handle(release("space")), Decision::Suppress);
}

#[test]
fn inner_dual_role_can_tap_but_marks_outer_used() {
    let mut engine = nested_dual_role_engine();

    engine.handle(Input::Modifier {
        modifier: Modifier::Shift,
        kind: EventKind::Down,
    });
    engine.handle(press("space"));
    engine.handle(press("tab"));
    assert_eq!(
        engine.handle(release("tab")),
        Decision::Trigger(Action::SendKeys("shift+tab".parse().unwrap()))
    );
    assert_eq!(engine.handle(release("space")), Decision::Suppress);
}

#[test]
fn releasing_outer_dual_role_first_leaves_inner_layer_usable() {
    let mut engine = nested_dual_role_engine();

    engine.handle(press("space"));
    engine.handle(press("tab"));
    assert_eq!(engine.handle(release("space")), Decision::Suppress);
    assert_eq!(
        engine.handle(press("f")),
        Decision::Trigger(Action::LaunchApp("Nav".into()))
    );
}

#[test]
fn hyper_then_dual_role_allows_inner_tap_and_marks_hyper_used() {
    let mut engine = dual_role_engine();

    engine.handle(press("caps_lock"));
    engine.handle(press("space"));
    assert_eq!(
        engine.handle(release("space")),
        Decision::Trigger(Action::SendKeys("space".parse().unwrap()))
    );
    assert_eq!(engine.handle(release("caps_lock")), Decision::Suppress);
}

#[test]
fn dual_role_then_hyper_allows_inner_hyper_tap_and_marks_dual_role_used() {
    let mut engine = dual_role_engine();

    engine.handle(press("space"));
    engine.handle(press("caps_lock"));
    assert_eq!(
        engine.handle(release("caps_lock")),
        Decision::Trigger(Action::SendKeys("escape".parse().unwrap()))
    );
    assert_eq!(engine.handle(release("space")), Decision::Suppress);
}

#[test]
fn replacing_config_clears_released_dual_role_bookkeeping() {
    let mut engine = dual_role_engine();
    engine.handle(press("space"));
    engine.handle(release("space"));

    let without_dual_role = Config::from_toml("").unwrap().compile().unwrap();
    engine.replace_config(without_dual_role);
    let readded = Config::from_toml(
        r#"
        [[dual_role]]
        key = "space"
        tap = "space"
        hold_modifier = "leader"
        "#,
    )
    .unwrap()
    .compile()
    .unwrap();
    engine.replace_config(readded);

    assert_eq!(engine.handle(release("space")), Decision::Pass);
}

#[test]
fn lost_ordinary_key_up_does_not_permanently_block_idle_reload() {
    let mut engine = dual_role_engine();

    assert_eq!(engine.handle(press("b")), Decision::Pass);
    assert!(engine.is_idle());
}
