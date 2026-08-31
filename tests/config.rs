use kiwi_keymapper::config::{Action, Config};

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
fn parses_portable_hyper_and_binding_actions() {
    let config = Config::from_toml(EXAMPLE).expect("example config should parse");
    let compiled = config.compile().expect("example config should validate");

    assert_eq!(compiled.hyper.key.as_str(), "caps_lock");
    assert_eq!(compiled.hyper.tap.to_string(), "escape");
    assert_eq!(
        compiled.bindings.get("hyper+t"),
        Some(&Action::LaunchApp("Ghostty".into()))
    );
    assert_eq!(
        compiled.bindings.get("hyper+a"),
        Some(&Action::SendKeys("control+a".parse().unwrap()))
    );
    assert!(!compiled.bindings.contains_key("left_option+h"));
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
