#![cfg(target_os = "macos")]

use std::path::Path;

use kiwi_keymapper::{
    key::Modifier,
    macos::{LABEL, key_to_keycode, keycode_to_key, launch_agent_plist, modifier_for_keycode},
};

#[test]
fn maps_common_ansi_and_navigation_keys_both_ways() {
    for (name, code) in [("a", 0_u16), ("t", 17), ("escape", 53), ("left", 123)] {
        assert_eq!(keycode_to_key(code).unwrap().as_str(), name);
        assert_eq!(key_to_keycode(&name.parse().unwrap()).unwrap(), code);
    }
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
