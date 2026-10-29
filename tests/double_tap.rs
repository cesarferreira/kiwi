use std::time::Duration;

use kiwi_keymapper::{
    config::{Action, AppAction, AppBehavior, Config},
    engine::{DOUBLE_TAP_GAP, DOUBLE_TAP_HOLD, Decision, Engine, EventKind, Input},
    key::{Chord, Modifier},
};

fn launch(target: &str) -> Action {
    Action::App(AppAction {
        target: target.into(),
        behavior: AppBehavior::Launch,
    })
}

fn engine(bindings: &str) -> Engine {
    Engine::new(
        Config::from_toml(&format!("[bindings]\n{bindings}"))
            .unwrap()
            .compile()
            .unwrap(),
    )
}

fn modifier(modifier: Modifier, kind: EventKind) -> Input {
    Input::Modifier { modifier, kind }
}

fn key(name: &str, kind: EventKind) -> Input {
    Input::Key {
        key: name.parse().unwrap(),
        kind,
        repeat: false,
    }
}

fn ms(value: u64) -> Duration {
    Duration::from_millis(value)
}

/// Taps `which` twice: press at `start`, each press lasting `hold`, with `gap`
/// between the first release and the second press. Returns every decision.
fn double_tap(
    engine: &mut Engine,
    which: Modifier,
    start: u64,
    hold: u64,
    gap: u64,
) -> Vec<Decision> {
    let second = start + hold + gap;
    vec![
        engine.handle_at(modifier(which, EventKind::Down), ms(start)),
        engine.handle_at(modifier(which, EventKind::Up), ms(start + hold)),
        engine.handle_at(modifier(which, EventKind::Down), ms(second)),
        engine.handle_at(modifier(which, EventKind::Up), ms(second + hold)),
    ]
}

#[test]
fn double_syntax_round_trips_and_rejects_misuse() {
    let chord: Chord = "double+right_cmd".parse().unwrap();
    assert_eq!(chord.to_string(), "double+right_command");
    assert_eq!(chord.modifiers, vec![Modifier::RightCommand]);
    assert!(chord.key.is_double_tap());

    for invalid in [
        "double+hyper",
        "double+a",
        "double+right_command+a",
        "right_command+double",
        "double",
        "right_command+double_tap",
    ] {
        assert!(
            invalid.parse::<Chord>().is_err(),
            "{invalid} should be rejected"
        );
    }
}

#[test]
fn quick_double_tap_triggers_once_and_passes_every_modifier_event() {
    let mut engine = engine(r#""double+right_command" = { app = "Raycast" }"#);

    let decisions = double_tap(&mut engine, Modifier::RightCommand, 0, 80, 120);

    assert_eq!(
        decisions,
        vec![
            Decision::Pass,
            Decision::Pass,
            Decision::Pass,
            Decision::PassAndTrigger(launch("Raycast")),
        ]
    );
    assert_eq!(
        engine.last_virtual_chord().unwrap().to_string(),
        "double+right_command"
    );
}

#[test]
fn a_third_tap_starts_a_new_double_tap() {
    let mut engine = engine(r#""double+right_command" = { app = "Raycast" }"#);
    double_tap(&mut engine, Modifier::RightCommand, 0, 50, 50);

    assert_eq!(
        engine.handle_at(modifier(Modifier::RightCommand, EventKind::Down), ms(250)),
        Decision::Pass
    );
    assert_eq!(
        engine.handle_at(modifier(Modifier::RightCommand, EventKind::Up), ms(300)),
        Decision::Pass,
        "the third tap is only the first half of the next double-tap"
    );
}

#[test]
fn slow_holds_long_gaps_and_other_keys_cancel_the_double_tap() {
    let bindings = r#""double+right_command" = { app = "Raycast" }"#;
    let hold = DOUBLE_TAP_HOLD.as_millis() as u64;
    let gap = DOUBLE_TAP_GAP.as_millis() as u64;

    let mut slow_hold = engine(bindings);
    assert!(
        !double_tap(&mut slow_hold, Modifier::RightCommand, 0, hold + 1, 50)
            .iter()
            .any(|decision| matches!(decision, Decision::PassAndTrigger(_)))
    );

    let mut long_gap = engine(bindings);
    assert!(
        !double_tap(&mut long_gap, Modifier::RightCommand, 0, 50, gap + 1)
            .iter()
            .any(|decision| matches!(decision, Decision::PassAndTrigger(_)))
    );

    let mut at_limits = engine(bindings);
    assert!(matches!(
        double_tap(&mut at_limits, Modifier::RightCommand, 0, hold, gap)[3],
        Decision::PassAndTrigger(_)
    ));

    // Command+C between the taps is a shortcut, not a tap.
    let mut shortcut = engine(bindings);
    shortcut.handle_at(modifier(Modifier::RightCommand, EventKind::Down), ms(0));
    shortcut.handle_at(key("c", EventKind::Down), ms(20));
    shortcut.handle_at(key("c", EventKind::Up), ms(40));
    shortcut.handle_at(modifier(Modifier::RightCommand, EventKind::Up), ms(60));
    shortcut.handle_at(modifier(Modifier::RightCommand, EventKind::Down), ms(100));
    assert_eq!(
        shortcut.handle_at(modifier(Modifier::RightCommand, EventKind::Up), ms(150)),
        Decision::Pass
    );

    // A key typed between two otherwise valid taps resets the sequence.
    let mut typed_between = engine(bindings);
    typed_between.handle_at(modifier(Modifier::RightCommand, EventKind::Down), ms(0));
    typed_between.handle_at(modifier(Modifier::RightCommand, EventKind::Up), ms(50));
    typed_between.handle_at(key("a", EventKind::Down), ms(80));
    typed_between.handle_at(key("a", EventKind::Up), ms(90));
    typed_between.handle_at(modifier(Modifier::RightCommand, EventKind::Down), ms(100));
    assert_eq!(
        typed_between.handle_at(modifier(Modifier::RightCommand, EventKind::Up), ms(150)),
        Decision::Pass
    );
}

#[test]
fn other_modifiers_held_or_mixed_sides_do_not_count() {
    let bindings = r#""double+right_command" = { app = "Raycast" }"#;

    let mut mixed_sides = engine(bindings);
    mixed_sides.handle_at(modifier(Modifier::LeftCommand, EventKind::Down), ms(0));
    mixed_sides.handle_at(modifier(Modifier::LeftCommand, EventKind::Up), ms(50));
    mixed_sides.handle_at(modifier(Modifier::RightCommand, EventKind::Down), ms(100));
    assert_eq!(
        mixed_sides.handle_at(modifier(Modifier::RightCommand, EventKind::Up), ms(150)),
        Decision::Pass
    );

    let mut with_shift = engine(bindings);
    with_shift.handle_at(modifier(Modifier::LeftShift, EventKind::Down), ms(0));
    let decisions = double_tap(&mut with_shift, Modifier::RightCommand, 10, 50, 50);
    assert!(decisions.iter().all(|decision| *decision == Decision::Pass));

    let mut under_hyper = engine(bindings);
    under_hyper.handle_at(key("caps_lock", EventKind::Down), ms(0));
    let decisions = double_tap(&mut under_hyper, Modifier::RightCommand, 10, 50, 50);
    assert!(decisions.iter().all(|decision| *decision == Decision::Pass));
}

#[test]
fn generic_modifier_matches_either_side_and_side_specific_wins() {
    let mut generic = engine(r#""double+command" = { app = "Generic" }"#);
    assert_eq!(
        double_tap(&mut generic, Modifier::LeftCommand, 0, 50, 50)[3],
        Decision::PassAndTrigger(launch("Generic"))
    );

    let mut both = engine(
        r#"
        "double+command" = { app = "Generic" }
        "double+right_command" = { app = "Right" }
        "#,
    );
    assert_eq!(
        double_tap(&mut both, Modifier::RightCommand, 0, 50, 50)[3],
        Decision::PassAndTrigger(launch("Right"))
    );
    assert_eq!(
        double_tap(&mut both, Modifier::LeftCommand, 1000, 50, 50)[3],
        Decision::PassAndTrigger(launch("Generic"))
    );
}

#[test]
fn unbound_double_tap_passes_and_cannot_be_emitted() {
    let mut unbound = engine("");
    assert!(
        double_tap(&mut unbound, Modifier::RightCommand, 0, 50, 50)
            .iter()
            .all(|decision| *decision == Decision::Pass)
    );

    let error = Config::from_toml(
        r#"[bindings]
        "hyper+k" = { keys = "double+right_command" }"#,
    )
    .unwrap()
    .compile()
    .unwrap_err()
    .to_string();
    assert!(error.contains("hyper+k"), "{error}");

    let error = Config::from_toml("[hyper]\ntap = \"double+right_command\"")
        .unwrap()
        .compile()
        .unwrap_err();
    assert!(format!("{error:#}").contains("keyboard keys"), "{error:#}");
}

#[test]
fn double_tap_bindings_are_listed_but_not_in_the_hyper_cheatsheet() {
    let config = Config::from_toml(
        r#"[bindings]
        "double+right_cmd" = { app = "Raycast" }
        "hyper+t" = { app = "Ghostty" }"#,
    )
    .unwrap()
    .compile()
    .unwrap();

    assert!(config.bindings.contains_key("double+right_command"));
    assert_eq!(config.hyper_binding_summary().len(), 1);
}
