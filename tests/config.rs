use kiwi_keymapper::config::{
    Action, AppAction, AppBehavior, Config, FeedbackPolicy, FeedbackStyle,
};

const EXAMPLE: &str = r#"
[hyper]
key = "caps_lock"
tap = "escape"
modifiers = ["command", "control", "option", "shift"]

[bindings]
"hyper+t" = { app = "Ghostty" }
"hyper+s" = { app = "Slack" }
"hyper+a" = { keys = "control+a" }
"left_option+h" = { keys = "left", enabled = false }
"#;

#[test]
fn ui_feedback_defaults_to_errors_and_notification() {
    let compiled = Config::from_toml("").unwrap().compile().unwrap();

    assert_eq!(compiled.ui.feedback, FeedbackPolicy::Errors);
    assert_eq!(compiled.ui.style, FeedbackStyle::Notification);
}

#[test]
fn partial_and_empty_ui_tables_keep_field_defaults() {
    for contents in [
        "[ui]",
        r#"[ui]
feedback = "all""#,
        r#"[ui]
style = "notification""#,
    ] {
        let compiled = Config::from_toml(contents).unwrap().compile().unwrap();
        let expected_feedback = if contents.contains("feedback") {
            FeedbackPolicy::All
        } else {
            FeedbackPolicy::Errors
        };

        assert_eq!(compiled.ui.feedback, expected_feedback);
        assert_eq!(compiled.ui.style, FeedbackStyle::Notification);
    }
}

#[test]
fn parses_every_supported_ui_feedback_policy() {
    for (value, expected) in [
        ("off", FeedbackPolicy::Off),
        ("errors", FeedbackPolicy::Errors),
        ("all", FeedbackPolicy::All),
    ] {
        let compiled = Config::from_toml(&format!(
            r#"
            [ui]
            feedback = "{value}"
            style = "notification"
            "#
        ))
        .unwrap()
        .compile()
        .unwrap();

        assert_eq!(compiled.ui.feedback, expected);
        assert_eq!(compiled.ui.style, FeedbackStyle::Notification);
    }
}

#[test]
fn rejects_unknown_ui_feedback_and_style_values() {
    for (contents, expected) in [
        (
            r#"[ui]
feedback = "sometimes""#,
            "unknown feedback",
        ),
        (
            r#"[ui]
style = "hud""#,
            "unknown feedback style",
        ),
    ] {
        let error = format!("{:#}", Config::from_toml(contents).unwrap_err());
        assert!(
            error.contains(expected),
            "expected `{expected}` in `{error}`"
        );
    }
}

#[test]
fn parses_portable_hyper_and_binding_actions() {
    let config = Config::from_toml(EXAMPLE).expect("example config should parse");
    let compiled = config.compile().expect("example config should validate");

    assert_eq!(compiled.hyper.key.as_str(), "caps_lock");
    assert_eq!(compiled.hyper.tap.to_string(), "escape");
    assert_eq!(
        compiled.bindings.get("hyper+t"),
        Some(&Action::App(AppAction {
            target: "Ghostty".into(),
            behavior: AppBehavior::Launch,
        }))
    );
    assert_eq!(
        compiled.bindings.get("hyper+a"),
        Some(&Action::SendKeys("control+a".parse().unwrap()))
    );
    assert!(!compiled.bindings.contains_key("left_option+h"));
}

#[test]
fn compiles_app_behaviors_and_defaults_to_launch() {
    let config = Config::from_toml(
        r#"
        [bindings]
        "hyper+l" = { app = "Ghostty" }
        "hyper+h" = { app = "Ghostty", behavior = "hide" }
        "hyper+c" = { app = "Ghostty", behavior = "cycle" }
        "hyper+n" = { app = "Ghostty", behavior = "new_window" }
        "hyper+t" = { app = "Ghostty", behavior = "toggle" }
        "#,
    )
    .unwrap()
    .compile()
    .unwrap();

    for (shortcut, behavior) in [
        ("hyper+l", AppBehavior::Launch),
        ("hyper+h", AppBehavior::Hide),
        ("hyper+c", AppBehavior::Cycle),
        ("hyper+n", AppBehavior::NewWindow),
        ("hyper+t", AppBehavior::Toggle),
    ] {
        assert_eq!(
            config.bindings.get(shortcut),
            Some(&Action::App(AppAction {
                target: "Ghostty".into(),
                behavior,
            }))
        );
    }
}

#[test]
fn rejects_behavior_without_app_and_unknown_behavior() {
    for (binding, expected) in [
        (
            r#""hyper+u" = { url = "https://example.com", behavior = "hide" }"#,
            "`behavior` is only valid with `app`",
        ),
        (
            r#""hyper+c" = { command = "echo hi", behavior = "cycle" }"#,
            "`behavior` is only valid with `app`",
        ),
        (
            r#""hyper+k" = { keys = "control+a", behavior = "new_window" }"#,
            "`behavior` is only valid with `app`",
        ),
        (
            r#""hyper+t" = { app = "Ghostty", behavior = "minimize" }"#,
            "unknown app behavior `minimize`; expected `launch`, `toggle`, `hide`, `cycle`, or `new_window`",
        ),
    ] {
        let config = Config::from_toml(&format!("[bindings]\n{binding}\n")).unwrap();
        let error = config.compile().unwrap_err().to_string();
        assert!(
            error.contains(expected),
            "expected `{expected}` in `{error}` for {binding}"
        );
    }
}

#[test]
fn rejects_a_binding_with_more_than_one_action() {
    let config = Config::from_toml(
        r#"
        [bindings]
        "hyper+t" = { app = "Ghostty", keys = "command+t" }
        "#,
    )
    .unwrap();

    let error = config.compile().unwrap_err().to_string();

    assert!(error.contains("hyper+t"));
    assert!(error.contains("exactly one"));
}

#[test]
fn rejects_unknown_keys_with_the_binding_name() {
    let config = Config::from_toml(
        r#"
        [bindings]
        "hyper+definitely_not_a_key" = { app = "Ghostty" }
        "#,
    )
    .unwrap();

    let error = config.compile().unwrap_err().to_string();

    assert!(error.contains("hyper+definitely_not_a_key"));
    assert!(error.contains("unknown key"));
}

#[test]
fn rejects_bindings_that_normalize_to_the_same_chord() {
    let config = Config::from_toml(
        r#"
        [bindings]
        "hyper+shift+a" = { app = "Ghostty" }
        "shift+hyper+a" = { app = "Slack" }
        "#,
    )
    .unwrap();

    let error = config.compile().unwrap_err().to_string();

    assert!(error.contains("duplicate binding"));
    assert!(error.contains("hyper+shift+a"));
}

#[test]
fn rejects_virtual_hyper_in_a_synthetic_keystroke() {
    let config = Config::from_toml(
        r#"
        [bindings]
        "hyper+x" = { keys = "hyper+y" }
        "#,
    )
    .unwrap();

    let error = config.compile().unwrap_err().to_string();

    assert!(error.contains("hyper+x"));
    assert!(error.contains("virtual `hyper`"));
}
