pub mod config;
pub mod conflicts;
pub mod engine;
pub mod key;

pub const DEFAULT_CONFIG: &str = r#"# kiwi configuration
# Chords use `+`; accepted aliases include cmd, ctrl, alt, and esc.

[hyper]
key = "caps_lock"
tap = "escape"
modifiers = ["command", "control", "option", "shift"]

[ui]
feedback = "errors"
style = "notification"

[bindings]
"hyper+t" = { app = "Ghostty" }
"hyper+s" = { app = "Slack" }
"hyper+a" = { keys = "control+a" }

# Enable these to reproduce the optional Karabiner navigation layer.
"left_option+h" = { keys = "left", enabled = false }
"left_option+j" = { keys = "down", enabled = false }
"left_option+k" = { keys = "up", enabled = false }
"left_option+l" = { keys = "right", enabled = false }

# Other supported actions:
# "hyper+b" = { url = "https://example.com" }
# "hyper+c" = { command = "open -a Calendar" }
"#;

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "macos")]
pub(crate) mod reload;
