#![cfg(target_os = "macos")]

use std::{path::Path, process::Command};

use kiwi_keymapper::{
    config::{AppAction, AppBehavior},
    key::Modifier,
    macos::{
        LABEL,
        app_controller::{TargetKind, classify_target, command_for},
        key_to_keycode, keycode_to_key, launch_agent_plist, modifier_for_keycode,
    },
};

#[test]
fn maps_common_ansi_and_navigation_keys_both_ways() {
    for (name, code) in [("a", 0_u16), ("t", 17), ("escape", 53), ("left", 123)] {
        assert_eq!(keycode_to_key(code).unwrap().as_str(), name);
        assert_eq!(key_to_keycode(&name.parse().unwrap()).unwrap(), code);
    }
}

#[test]
fn hardware_keys_clone_without_copying_their_static_name() {
    let key = keycode_to_key(0).unwrap();
    let cloned = key.clone();

    assert_eq!(key.as_str(), "a");
    assert_eq!(key.as_str().as_ptr(), cloned.as_str().as_ptr());
}

#[test]
fn preserves_the_side_of_physical_modifiers() {
    assert_eq!(modifier_for_keycode(58), Some(Modifier::LeftOption));
    assert_eq!(modifier_for_keycode(61), Some(Modifier::RightOption));
    assert_eq!(modifier_for_keycode(56), Some(Modifier::LeftShift));
}

#[test]
fn launch_agent_uses_explicit_paths_and_escapes_xml() {
    let plist = launch_agent_plist(
        Path::new("/Applications/Kiwi & Friends/kiwi"),
        Path::new("/Users/cesar/.config/kiwi/config.toml"),
        Path::new("/Users/cesar/Library/Logs/kiwi.log"),
    );

    assert_eq!(LABEL, "io.github.cesarferreira.kiwi");
    assert!(plist.contains("/Applications/Kiwi &amp; Friends/kiwi"));
    assert!(plist.contains("<string>--config</string>"));
    assert!(plist.contains("<string>run</string>"));
    assert!(plist.contains("<key>RunAtLoad</key>\n  <true/>"));
}

#[test]
fn classifies_app_names_absolute_paths_and_bundle_identifiers() {
    assert_eq!(classify_target("Ghostty"), TargetKind::Name);
    assert_eq!(classify_target("Ghostty.Preview"), TargetKind::Name);
    assert_eq!(
        classify_target("/Applications/Ghostty.app"),
        TargetKind::Path
    );
    assert_eq!(
        classify_target("com.mitchellh.ghostty"),
        TargetKind::BundleIdentifier
    );
    assert_eq!(classify_target("io.ghostty"), TargetKind::BundleIdentifier);
    assert_eq!(
        classify_target("company.product.editor"),
        TargetKind::BundleIdentifier
    );
}

#[test]
fn launch_and_new_window_commands_support_every_target_form() {
    for (target, behavior, expected_args) in [
        ("Ghostty", AppBehavior::Launch, vec!["-a", "Ghostty"]),
        (
            "/Applications/Ghostty.app",
            AppBehavior::Launch,
            vec!["-a", "/Applications/Ghostty.app"],
        ),
        (
            "com.mitchellh.ghostty",
            AppBehavior::Launch,
            vec!["-b", "com.mitchellh.ghostty"],
        ),
        ("Ghostty", AppBehavior::NewWindow, vec!["-na", "Ghostty"]),
        (
            "/Applications/Ghostty.app",
            AppBehavior::NewWindow,
            vec!["-na", "/Applications/Ghostty.app"],
        ),
        (
            "com.mitchellh.ghostty",
            AppBehavior::NewWindow,
            vec!["-n", "-b", "com.mitchellh.ghostty"],
        ),
    ] {
        let command = command_for(&AppAction {
            target: target.into(),
            behavior,
        });
        assert_eq!(command.program, "/usr/bin/open");
        assert_eq!(command.args, expected_args);
    }
}

#[test]
fn hide_and_cycle_pass_classification_and_target_as_separate_script_arguments() {
    let target = r#"Ghostty"; error "injected""#;

    for behavior in [AppBehavior::Hide, AppBehavior::Cycle] {
        let command = command_for(&AppAction {
            target: target.into(),
            behavior,
        });

        assert_eq!(command.program, "/usr/bin/osascript");
        assert_eq!(command.args[0], "-e");
        assert!(!command.args[1].contains(target));
        assert_eq!(command.args[2], "--");
        assert_eq!(
            command.args[3],
            match behavior {
                AppBehavior::Hide => "hide",
                AppBehavior::Cycle => "cycle",
                _ => unreachable!(),
            }
        );
        assert_eq!(command.args[4], "name");
        assert_eq!(command.args[5], target);
        assert!(command.args[1].contains("bundle identifier"));
        assert!(command.args[1].contains("application is not running"));
    }
}

#[test]
fn script_resolves_only_running_targets_without_path_to_application() {
    let command = command_for(&AppAction {
        target: "Ghostty".into(),
        behavior: AppBehavior::Hide,
    });
    let script = &command.args[1];

    assert!(!script.contains("path to application"));
    assert!(script.contains("info for targetFile"));
    assert!(script.contains("whose bundle identifier is appTarget"));
    assert!(script.contains("whose name is appTarget"));
    assert!(script.contains("application is not running"));
}

#[test]
fn script_arguments_preserve_name_path_and_bundle_identifier_targets() {
    for (target, expected_kind) in [
        ("Ghostty", "name"),
        ("/Applications/Ghostty Preview.app", "path"),
        ("com.mitchellh.ghostty", "bundle_id"),
    ] {
        let command = command_for(&AppAction {
            target: target.into(),
            behavior: AppBehavior::Hide,
        });
        assert_eq!(command.args[2], "--");
        assert_eq!(command.args[3], "hide");
        assert_eq!(command.args[4], expected_kind);
        assert_eq!(command.args[5], target);
    }
}

#[test]
fn cycle_uses_target_process_windows_and_ax_actions_without_global_keystrokes() {
    let command = command_for(&AppAction {
        target: "Ghostty".into(),
        behavior: AppBehavior::Cycle,
    });
    let script = &command.args[1];

    assert!(!script.contains("key code 50"));
    assert!(!script.contains("keystroke"));
    assert!(script.contains("windows of targetProcess"));
    assert!(script.contains(r#"attribute "AXMain""#));
    assert!(script.contains(r#"attribute "AXFocused""#));
    let raise = script.find(r#"perform action "AXRaise""#).unwrap();
    let frontmost = script
        .find("set frontmost of targetProcess to true")
        .unwrap();
    assert!(raise < frontmost);
}

#[test]
fn static_app_action_script_compiles() {
    let command = command_for(&AppAction {
        target: "Ghostty".into(),
        behavior: AppBehavior::Cycle,
    });
    let output = Command::new("/usr/bin/osacompile")
        .args(["-o", "/dev/null", "-e", &command.args[1]])
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
