use std::{fs, process::Command};

use keyweave::config::Config;

#[test]
fn shipped_default_config_is_valid_and_matches_the_migration_example() {
    let compiled = Config::from_toml(keyweave::DEFAULT_CONFIG)
        .unwrap()
        .compile()
        .unwrap();

    assert!(compiled.bindings.contains_key("hyper+t"));
    assert!(compiled.bindings.contains_key("hyper+s"));
    assert!(compiled.bindings.contains_key("hyper+a"));
}

#[test]
fn validate_command_checks_the_selected_config() {
    let path = std::env::temp_dir().join(format!("keyweave-test-{}.toml", std::process::id()));
    fs::write(&path, keyweave::DEFAULT_CONFIG).unwrap();

    let output = Command::new(env!("CARGO_BIN_EXE_keyweave"))
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
fn default_config_path_is_dotfiles_friendly_on_macos() {
    let output = Command::new(env!("CARGO_BIN_EXE_keyweave"))
        .arg("config-path")
        .output()
        .unwrap();

    assert!(output.status.success());
    assert!(
        String::from_utf8(output.stdout)
            .unwrap()
            .trim_end()
            .ends_with("/.config/keyweave/config.toml")
    );
}
