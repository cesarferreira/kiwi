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

#[test]
fn compiles_named_dual_role_modifiers_in_bindings() {
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

    assert_eq!(config.dual_roles.len(), 1);
    assert_eq!(config.dual_roles[0].key.as_str(), "space");
    assert_eq!(config.dual_roles[0].tap.to_string(), "space");
    assert_eq!(config.dual_roles[0].hold_modifier, "leader");
    assert!(config.bindings.contains_key("leader+f"));
    assert!(config.bindings.contains_key("hyper+leader+t"));
}

#[test]
fn rejects_invalid_dual_role_declarations() {
    let cases = [
        (
            r#"[[dual_role]]
key = "not_a_key"
tap = "space"
hold_modifier = "leader""#,
            "invalid dual-role key",
        ),
        (
            r#"[[dual_role]]
key = "space"
tap = "not_a_key"
hold_modifier = "leader""#,
            "invalid dual-role tap",
        ),
        (
            r#"[[dual_role]]
key = "space"
tap = "space"
hold_modifier = "hyper""#,
            "reserved",
        ),
        (
            r#"[[dual_role]]
key = "space"
tap = "space"
hold_modifier = "shift""#,
            "collides",
        ),
        (
            r#"[[dual_role]]
key = "space"
tap = "space"
hold_modifier = "space""#,
            "collides",
        ),
        (
            r#"[[dual_role]]
key = "space"
tap = "space"
hold_modifier = "leader+mode""#,
            "valid modifier name",
        ),
        (
            r#"[[dual_role]]
key = "space"
tap = "space"
hold_modifier = "leader"

[[dual_role]]
key = "space"
tap = "tab"
hold_modifier = "nav""#,
            "duplicate dual-role key",
        ),
        (
            r#"[[dual_role]]
key = "space"
tap = "space"
hold_modifier = "leader"

[[dual_role]]
key = "tab"
tap = "tab"
hold_modifier = "leader""#,
            "duplicate hold modifier",
        ),
        (
            r#"[[dual_role]]
key = "caps_lock"
tap = "space"
hold_modifier = "leader""#,
            "duplicates the hyper key",
        ),
    ];

    for (contents, expected) in cases {
        let error = Config::from_toml(contents)
            .unwrap()
            .compile()
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(expected),
            "expected `{expected}` in `{error}` for:\n{contents}"
        );
    }
}

#[test]
fn rejects_undefined_named_modifier_in_binding() {
    let config = Config::from_toml(
        r#"
        [bindings]
        "leader+f" = { app = "Finder" }
        "#,
    )
    .unwrap();

    let error = config.compile().unwrap_err().to_string();

    assert!(error.contains("leader+f"));
    assert!(error.contains("unknown modifier `leader`"));
}

#[test]
fn named_modifier_normalization_is_deterministic() {
    let config = Config::from_toml(
        r#"
        [[dual_role]]
        key = "space"
        tap = "space"
        hold_modifier = "leader-mode"

        [bindings]
        "Shift+leader-mode+F" = { app = "Finder" }
        "#,
    )
    .unwrap()
    .compile()
    .unwrap();

    assert!(config.bindings.contains_key("shift+leader_mode+f"));
}
