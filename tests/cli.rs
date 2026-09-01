use std::{
    fs,
    process::Command,
    sync::atomic::{AtomicUsize, Ordering},
};

use kiwi_keymapper::config::Config;

#[test]
fn shipped_default_config_is_valid_and_matches_the_migration_example() {
    let compiled = Config::from_toml(kiwi_keymapper::DEFAULT_CONFIG)
        .unwrap()
        .compile()
        .unwrap();

    assert!(compiled.bindings.contains_key("hyper+t"));
    assert!(compiled.bindings.contains_key("hyper+s"));
    assert!(compiled.bindings.contains_key("hyper+a"));
}

#[test]
fn validate_command_checks_the_selected_config() {
    let path = std::env::temp_dir().join(format!("kiwi-test-{}.toml", std::process::id()));
    fs::write(&path, kiwi_keymapper::DEFAULT_CONFIG).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .args(["--config", path.to_str().unwrap(), "validate"])
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();

    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("config is valid")
    );
}

#[test]
fn validate_rejects_cheatsheet_with_more_than_64_enabled_hyper_bindings() {
    let mut config = String::from("[ui]\ncheatsheet = true\n[bindings]\n");
    for key in ('a'..='z').chain('0'..='9') {
        config.push_str(&format!("\"hyper+{key}\" = {{ app = \"App{key}\" }}\n"));
    }
    for number in 1..=20 {
        config.push_str(&format!(
            "\"hyper+f{number}\" = {{ app = \"Fn{number}\" }}\n"
        ));
    }
    for extra in [
        "escape", "enter", "tab", "space", "delete", "left", "right", "up", "down",
    ] {
        config.push_str(&format!("\"hyper+{extra}\" = {{ app = \"App{extra}\" }}\n"));
    }
    let path =
        std::env::temp_dir().join(format!("kiwi-cheatsheet-limit-{}.toml", std::process::id()));
    fs::write(&path, config).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .args(["--config", path.to_str().unwrap(), "validate"])
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("cheatsheet"), "{stderr}");
    assert!(stderr.contains("64"), "{stderr}");
}

#[test]
fn default_config_path_is_dotfiles_friendly_on_macos() {
    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .arg("config-path")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .ends_with("/.config/kiwi/config.toml")
    );
}

#[test]
fn help_uses_the_kiwi_command_name() {
    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .arg("--help")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("Usage: kiwi")
    );
}

#[test]
fn help_exposes_start_and_stop_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(output.status.success());
    assert!(stdout.contains("  start"));
    assert!(stdout.contains("  stop"));
}

#[test]
fn hidden_cheatsheet_helper_parses_but_is_not_advertised() {
    let main = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(
        !String::from_utf8(main.stdout)
            .unwrap()
            .contains("__cheatsheet")
    );

    let helper = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .args(["__cheatsheet-overlay", "--help"])
        .output()
        .unwrap();
    assert!(
        helper.status.success(),
        "{}",
        String::from_utf8_lossy(&helper.stderr)
    );
}

#[test]
fn hidden_cheatsheet_helper_rejects_invalid_structured_input_without_opening_a_window() {
    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .arg("__cheatsheet-overlay")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap()
        .wait_with_output()
        .unwrap();

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("cheatsheet model"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn list_prints_enabled_shortcuts_as_an_aligned_table() {
    let path = std::env::temp_dir().join(format!("kiwi-list-test-{}.toml", std::process::id()));
    fs::write(
        &path,
        r#"
[bindings]
"hyper+u" = { url = "https://example.com" }
"hyper+p" = { command = "bluepods connect AirPods" }
"hyper+t" = { keys = "control+a" }
"hyper+a" = { app = "Arc" }
"hyper+x" = { app = "Disabled", enabled = false }
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .args(["--config", path.to_str().unwrap(), "list"])
        .output()
        .unwrap();
    fs::remove_file(path).unwrap();

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout,
        concat!(
            "4 shortcuts\n",
            "\n",
            "SHORTCUT  TYPE     ACTION\n",
            "hyper+a   app      Arc\n",
            "hyper+p   command  bluepods connect AirPods\n",
            "hyper+t   keys     control+a\n",
            "hyper+u   url      https://example.com\n",
        )
    );
}

#[test]
fn list_json_prints_stable_config_and_enabled_bindings() {
    let path =
        std::env::temp_dir().join(format!("kiwi-list-json-test-{}.toml", std::process::id()));
    fs::write(
        &path,
        r#"
[hyper]
key = "f19"
tap = "escape"
modifiers = ["command", "option"]

[ui]
feedback = "all"
style = "notification"

[bindings]
"hyper+u" = { url = "https://example.com" }
"hyper+p" = { command = "echo hi" }
"hyper+t" = { keys = "control+a" }
"hyper+a" = { app = "Arc" }
"hyper+x" = { app = "Disabled", enabled = false }
"#,
    )
    .unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .args([
            "--format",
            "json",
            "--config",
            path.to_str().unwrap(),
            "list",
        ])
        .output()
        .unwrap();
    fs::remove_file(&path).unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert_eq!(
        stdout,
        format!(
            concat!(
                "{{\"schema_version\":1,\"config_path\":\"{}\",",
                "\"hyper\":{{\"key\":\"f19\",\"tap\":\"escape\",",
                "\"modifiers\":[\"command\",\"option\"]}},",
                "\"bindings\":[",
                "{{\"shortcut\":\"hyper+a\",\"type\":\"app\",\"action\":\"Arc\"}},",
                "{{\"shortcut\":\"hyper+p\",\"type\":\"command\",\"action\":\"echo hi\"}},",
                "{{\"shortcut\":\"hyper+t\",\"type\":\"keys\",\"action\":\"control+a\"}},",
                "{{\"shortcut\":\"hyper+u\",\"type\":\"url\",\"action\":\"https://example.com\"}}",
                "]}}\n"
            ),
            path.display()
        )
    );
    assert!(!stdout.contains("\u{1b}["));
}

#[test]
fn list_text_and_json_distinguish_app_behaviors() {
    let (_, text) = run_conflicts(
        r#"
[bindings]
"hyper+l" = { app = "Ghostty" }
"hyper+h" = { app = "Ghostty", behavior = "hide" }
"hyper+c" = { app = "Ghostty", behavior = "cycle" }
"hyper+n" = { app = "Ghostty", behavior = "new_window" }
"hyper+t" = { app = "Ghostty", behavior = "toggle" }
"#,
        &["list"],
    );
    assert!(text.status.success());
    let stdout = String::from_utf8(text.stdout).unwrap();
    assert!(stdout.contains("hyper+l   app   Ghostty\n"));
    assert!(stdout.contains("hyper+h   app   Ghostty (hide)\n"));
    assert!(stdout.contains("hyper+c   app   Ghostty (cycle)\n"));
    assert!(stdout.contains("hyper+n   app   Ghostty (new window)\n"));
    assert!(stdout.contains("hyper+t   app   Ghostty (toggle)\n"));

    let (_, json) = run_conflicts(
        r#"
[bindings]
"hyper+h" = { app = "Ghostty", behavior = "hide" }
"hyper+c" = { app = "Ghostty", behavior = "cycle" }
"hyper+n" = { app = "Ghostty", behavior = "new_window" }
"hyper+t" = { app = "Ghostty", behavior = "toggle" }
"#,
        &["--format", "json", "list"],
    );
    assert!(json.status.success());
    let json: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    let bindings = json["bindings"].as_array().unwrap();
    assert_eq!(bindings[0]["type"], "app");
    assert_eq!(bindings[0]["action"], "Ghostty (cycle)");
    assert_eq!(bindings[1]["action"], "Ghostty (hide)");
    assert_eq!(bindings[2]["action"], "Ghostty (new window)");
    assert_eq!(bindings[3]["action"], "Ghostty (toggle)");
}

#[test]
fn help_exposes_the_list_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(output.status.success());
    assert!(stdout.contains("  list"));
}

#[test]
fn help_exposes_the_listen_command() {
    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .arg("--help")
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();

    assert!(output.status.success());
    assert!(stdout.contains("  listen"));
}

fn run_conflicts(config: &str, arguments: &[&str]) -> (std::path::PathBuf, std::process::Output) {
    static NEXT_ID: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "kiwi-conflicts-test-{}-{}.toml",
        std::process::id(),
        NEXT_ID.fetch_add(1, Ordering::Relaxed)
    ));
    fs::write(&path, config).unwrap();
    let mut command = Command::new(env!("CARGO_BIN_EXE_kiwi"));
    command.args(["--config", path.to_str().unwrap()]);
    command.args(arguments);
    let output = command.output().unwrap();
    fs::remove_file(&path).unwrap();
    (path, output)
}

#[test]
fn list_conflicts_prints_an_aligned_table_and_exits_one() {
    let (_, output) = run_conflicts(
        r#"
[bindings]
"command+space" = { app = "Finder" }
"command+shift+3" = { command = "capture" }
"command+tab" = { app = "Disabled", enabled = false }
"hyper+t" = { keys = "command+space" }
"#,
        &["list", "--conflicts"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        concat!(
            "2 conflicts\n",
            "\n",
            "SHORTCUT         TYPE     ACTION   CONFLICTS WITH\n",
            "command+shift+3  command  capture  macOS Screenshot entire screen\n",
            "command+space    app      Finder   macOS Spotlight\n",
        )
    );
}

#[test]
fn list_conflicts_matches_side_specific_physical_binding() {
    let (_, output) = run_conflicts(
        r#"
[bindings]
"left_command+space" = { app = "Finder" }
"#,
        &["list", "--conflicts"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("left_command+space")
    );
}

#[test]
fn list_conflicts_exits_zero_when_there_are_no_hits() {
    let (_, output) = run_conflicts(
        r#"
[bindings]
"hyper+t" = { app = "Ghostty" }
"command+space" = { app = "Disabled", enabled = false }
"#,
        &["list", "--conflicts"],
    );

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "No shortcut conflicts found.\n"
    );
}

#[test]
fn json_list_conflicts_adds_a_stable_conflicts_array_to_the_list_object() {
    let (path, output) = run_conflicts(
        r#"
[hyper]
key = "f19"
tap = "escape"
modifiers = ["command", "option"]

[bindings]
"command+space" = { app = "Finder" }
"#,
        &["list", "--conflicts", "--format", "json"],
    );

    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            concat!(
                "{{\"schema_version\":1,\"config_path\":\"{}\",",
                "\"hyper\":{{\"key\":\"f19\",\"tap\":\"escape\",",
                "\"modifiers\":[\"command\",\"option\"]}},",
                "\"bindings\":[",
                "{{\"shortcut\":\"command+space\",\"type\":\"app\",\"action\":\"Finder\"}}",
                "],",
                "\"conflicts\":[{{\"shortcut\":\"command+space\",\"type\":\"app\",",
                "\"action\":\"Finder\",\"source\":\"macos\",\"label\":\"Spotlight\",",
                "\"url\":\"https://support.apple.com/guide/mac-help/",
                "keyboard-shortcuts-mchlp2262/mac\"}}]}}\n"
            ),
            path.display()
        )
    );
}

#[test]
fn json_list_conflicts_reports_an_empty_array_and_exits_zero_without_hits() {
    let (path, output) = run_conflicts(
        r#"
[hyper]
key = "f19"
tap = "escape"
modifiers = ["command", "option"]

[bindings]
"hyper+t" = { app = "Ghostty" }
"#,
        &["list", "--conflicts", "--format", "json"],
    );

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        format!(
            concat!(
                "{{\"schema_version\":1,\"config_path\":\"{}\",",
                "\"hyper\":{{\"key\":\"f19\",\"tap\":\"escape\",",
                "\"modifiers\":[\"command\",\"option\"]}},",
                "\"bindings\":[",
                "{{\"shortcut\":\"hyper+t\",\"type\":\"app\",\"action\":\"Ghostty\"}}",
                "],\"conflicts\":[]}}\n"
            ),
            path.display()
        )
    );
}

#[test]
fn conflict_text_and_json_distinguish_app_behavior() {
    for (behavior, expected) in [("hide", "Finder (hide)"), ("toggle", "Finder (toggle)")] {
        let config = format!(
            r#"
[bindings]
"command+space" = {{ app = "Finder", behavior = "{behavior}" }}
"#
        );
        let (_, text) = run_conflicts(&config, &["list", "--conflicts"]);
        assert_eq!(text.status.code(), Some(1));
        assert!(String::from_utf8(text.stdout).unwrap().contains(expected));

        let (_, json) = run_conflicts(&config, &["list", "--conflicts", "--format", "json"]);
        assert_eq!(json.status.code(), Some(1));
        let json: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
        assert_eq!(json["bindings"][0]["type"], "app");
        assert_eq!(json["bindings"][0]["action"], expected);
        assert_eq!(json["conflicts"][0]["type"], "app");
        assert_eq!(json["conflicts"][0]["action"], expected);
    }
}

#[test]
fn ordinary_json_list_does_not_add_conflicts_array() {
    let (_, output) = run_conflicts(
        r#"
[bindings]
"command+space" = { app = "Finder" }
"#,
        &["--format", "json", "list"],
    );

    assert!(output.status.success());
    let json: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(json.get("conflicts").is_none());
}

#[test]
fn list_help_documents_the_conflicts_flag() {
    let output = Command::new(env!("CARGO_BIN_EXE_kiwi"))
        .args(["list", "--help"])
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .contains("--conflicts")
    );
}
