use kiwi_keymapper::{
    config::{Action, Config, WindowDirection},
    engine::{Decision, Engine, EventKind, Input},
    gestures::{Scroll, ScrollPhase},
};

fn engine() -> Engine {
    Engine::new(
        Config::from_toml(
            r#"[bindings]
"hyper+swipe_right" = { window = "next" }
"hyper+swipe_left" = { window = "previous" }
"#,
        )
        .unwrap()
        .compile()
        .unwrap(),
    )
}
fn hyper(engine: &mut Engine, kind: EventKind) -> Decision {
    engine.handle(Input::Key {
        key: "caps_lock".parse().unwrap(),
        kind,
        repeat: false,
    })
}
fn scroll(engine: &mut Engine, x: i64, y: i64, phase: ScrollPhase, momentum: bool) -> Decision {
    engine.handle(Input::Scroll(Scroll {
        x,
        y,
        phase,
        momentum,
        continuous: true,
    }))
}
#[test]
fn fires_once_per_swipe_ignores_momentum_and_does_not_emit_escape() {
    let mut e = engine();
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, 12, 0, ScrollPhase::Began, false),
        Decision::Suppress
    );
    assert_eq!(
        scroll(&mut e, 20, 0, ScrollPhase::Changed, false),
        Decision::Trigger(Action::Window(WindowDirection::Next))
    );
    assert_eq!(
        scroll(&mut e, 100, 0, ScrollPhase::Changed, false),
        Decision::Suppress
    );
    scroll(&mut e, 0, 0, ScrollPhase::Ended, false);
    assert_eq!(hyper(&mut e, EventKind::Up), Decision::Suppress);
    assert_eq!(
        scroll(&mut e, 100, 0, ScrollPhase::None, true),
        Decision::Suppress
    );
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, -40, 0, ScrollPhase::Began, false),
        Decision::Trigger(Action::Window(WindowDirection::Previous))
    );
}
#[test]
fn ordinary_scroll_vertical_motion_and_mid_scroll_hyper_do_not_trigger() {
    let mut e = engine();
    assert_eq!(
        scroll(&mut e, 100, 0, ScrollPhase::Began, false),
        Decision::Pass
    );
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, 100, 0, ScrollPhase::Changed, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 5, 70, ScrollPhase::Began, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 70, 0, ScrollPhase::Changed, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 70, 0, ScrollPhase::None, false),
        Decision::Pass
    );
}
#[test]
fn unbound_direction_and_phase_less_mouse_wheels_pass_through() {
    let mut e = Engine::new(
        Config::from_toml(
            r#"[bindings]
"hyper+swipe_right" = { window = "next" }
"#,
        )
        .unwrap()
        .compile()
        .unwrap(),
    );
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, -60, 0, ScrollPhase::Began, false),
        Decision::Pass
    );
    assert_eq!(
        e.handle(Input::Scroll(Scroll {
            x: 100,
            y: 0,
            phase: ScrollPhase::Began,
            momentum: false,
            continuous: false
        })),
        Decision::Pass
    );
}
#[test]
fn validates_gesture_and_window_actions() {
    for contents in [
        r#"[bindings]
"swipe_right" = { window = "next" }"#,
        r#"[bindings]
"hyper+x" = { keys = "swipe_right" }"#,
        r#"[bindings]
"hyper+x" = { window = "sideways" }"#,
        r#"[hyper]
key = "swipe_left""#,
        r#"[bindings]
"hyper+x" = { window = "next", command = "echo hi" }"#,
    ] {
        assert!(
            Config::from_toml(contents)
                .and_then(Config::compile)
                .is_err(),
            "{contents}"
        );
    }
    let cfg = Config::from_toml(
        r#"[bindings]
"hyper+swipe_right" = { window = "next" }
"hyper+tab" = { window = "previous" }
"#,
    )
    .unwrap()
    .compile()
    .unwrap();
    assert_eq!(cfg.hyper_binding_summary()[0].kind, "window");
}

#[test]
fn a_captured_swipe_does_not_swallow_later_mouse_wheel_events() {
    let mut e = engine();
    hyper(&mut e, EventKind::Down);
    scroll(&mut e, 60, 0, ScrollPhase::Began, false);
    scroll(&mut e, 0, 0, ScrollPhase::Ended, false);
    hyper(&mut e, EventKind::Up);
    assert_eq!(
        scroll(&mut e, 100, 0, ScrollPhase::None, false),
        Decision::Pass
    );
}
#[test]
fn releasing_then_repressing_hyper_cannot_arm_an_existing_swipe() {
    let mut e = engine();
    hyper(&mut e, EventKind::Down);
    scroll(&mut e, 5, 0, ScrollPhase::Began, false);
    hyper(&mut e, EventKind::Up);
    hyper(&mut e, EventKind::Down);
    assert!(!matches!(
        scroll(&mut e, 60, 0, ScrollPhase::Changed, false),
        Decision::Trigger(_)
    ));
}

fn vertical_engine() -> Engine {
    Engine::new(
        Config::from_toml(
            r#"[bindings]
"hyper+swipe_up" = { command = "snap full" }
"hyper+swipe_down" = { command = "snap almost" }
"hyper+swipe_right" = { window = "next" }
"#,
        )
        .unwrap()
        .compile()
        .unwrap(),
    )
}
#[test]
fn vertical_bindings_fire_once_ignore_momentum_and_do_not_emit_escape() {
    let mut e = vertical_engine();
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, 0, -12, ScrollPhase::Began, false),
        Decision::Suppress
    );
    assert_eq!(
        scroll(&mut e, 0, -20, ScrollPhase::Changed, false),
        Decision::Trigger(Action::RunCommand("snap full".into()))
    );
    assert_eq!(
        scroll(&mut e, 80, 0, ScrollPhase::Changed, false),
        Decision::Suppress
    );
    scroll(&mut e, 0, 0, ScrollPhase::Ended, false);
    assert_eq!(hyper(&mut e, EventKind::Up), Decision::Suppress);
    assert_eq!(
        scroll(&mut e, 0, -100, ScrollPhase::None, true),
        Decision::Suppress
    );
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, 0, 40, ScrollPhase::Began, false),
        Decision::Trigger(Action::RunCommand("snap almost".into()))
    );
}
#[test]
fn vertical_gestures_require_hyper_from_the_start_and_ignore_diagonals() {
    let mut e = vertical_engine();
    assert_eq!(
        scroll(&mut e, 0, -100, ScrollPhase::Began, false),
        Decision::Pass
    );
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, 0, -100, ScrollPhase::Changed, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 40, -40, ScrollPhase::Began, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 40, -40, ScrollPhase::Changed, false),
        Decision::Pass
    );
}
#[test]
fn unbound_vertical_direction_passes_and_cannot_turn_into_horizontal_action() {
    let mut e = Engine::new(
        Config::from_toml(
            r#"[bindings]
"hyper+swipe_up" = { command = "snap full" }
"hyper+swipe_right" = { window = "next" }
"#,
        )
        .unwrap()
        .compile()
        .unwrap(),
    );
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, 0, 70, ScrollPhase::Began, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 300, 0, ScrollPhase::Changed, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 0, -70, ScrollPhase::Began, false),
        Decision::Trigger(Action::RunCommand("snap full".into()))
    );
}
#[test]
fn vertical_gestures_cannot_be_bound_without_hyper_or_emitted_as_keys() {
    for (source, action) in [
        ("swipe_up", "command = 'snap full'"),
        ("hyper+x", "keys = 'swipe_down'"),
    ] {
        let contents = format!("[bindings]\n\"{source}\" = {{ {action} }}");
        assert!(
            Config::from_toml(&contents)
                .and_then(Config::compile)
                .is_err()
        );
    }
    let cfg = Config::from_toml(
        r#"[bindings]
"hyper+swipe_up" = { command = "snap full" }
"hyper+swipe_down" = { command = "snap almost" }
"#,
    )
    .unwrap()
    .compile()
    .unwrap();
    assert_eq!(cfg.hyper_binding_summary().len(), 2);
    assert!(
        cfg.hyper_binding_summary()
            .iter()
            .any(|row| row.key == "swipe_up" && row.action == "snap full")
    );
}

#[test]
fn provisional_capture_is_released_when_jitter_becomes_an_unbound_gesture() {
    let mut e = Engine::new(
        Config::from_toml(
            r#"[bindings]
"hyper+swipe_up" = { command = "snap full" }
"#,
        )
        .unwrap()
        .compile()
        .unwrap(),
    );
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, 0, -1, ScrollPhase::Began, false),
        Decision::Suppress
    );
    assert_eq!(
        scroll(&mut e, 40, 0, ScrollPhase::Changed, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 100, 0, ScrollPhase::None, true),
        Decision::Pass
    );
}
#[test]
fn diagonal_contact_releases_provisional_capture_and_never_later_triggers() {
    let mut e = vertical_engine();
    hyper(&mut e, EventKind::Down);
    assert_eq!(
        scroll(&mut e, 0, -1, ScrollPhase::Began, false),
        Decision::Suppress
    );
    assert_eq!(
        scroll(&mut e, 40, 40, ScrollPhase::Changed, false),
        Decision::Pass
    );
    assert_eq!(
        scroll(&mut e, 300, 0, ScrollPhase::Changed, false),
        Decision::Pass
    );
}
