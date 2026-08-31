<div align="center">
  <h1>keyweave</h1>

  <p><strong>Run portable macOS key mappings</strong></p>

  <p>
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
    <img alt="Edition" src="https://img.shields.io/badge/edition-2024-blue">
    <a href="https://crates.io/crates/keyweave"><img alt="crates.io" src="https://img.shields.io/crates/v/keyweave.svg"></a>
  </p>

  <p>
    <a href="#install">Install</a>
    &nbsp;·&nbsp;
    <a href="#quickstart">Quickstart</a>
    &nbsp;·&nbsp;
    <a href="#development">Development</a>
  </p>

</div>

---

`keyweave` replaces the common Karabiner-Elements + launcher combination with
one small native daemon. It can turn Caps Lock into Hyper when held and Escape
when tapped, launch applications, open URLs, run shell commands, and emit other
keyboard shortcuts.

- One declarative TOML configuration
- Global shortcuts available in every application
- Caps Lock → Hyper/Escape dual-role key
- App, URL, command, and key-sequence actions
- Generic and side-specific modifiers
- A LaunchAgent for automatic startup
- No Karabiner-Elements or Raycast dependency

<a id="install"></a>
## Requirements

- macOS
- Rust 1.85 or newer
- `~/.cargo/bin` on your `PATH`
- Accessibility permission for the installed `keyweave` binary
- A stable Apple code-signing identity

Check that macOS can see a signing identity:

```sh
security find-identity -p codesigning -v
```

`keyweave install` prefers a **Developer ID Application** identity, then an
**Apple Development** identity. A stable signature matters because macOS
Accessibility permission is tied to the identity of the executable.

<a id="quickstart"></a>
## Quick start

```sh
git clone https://github.com/cesarferreira/keyweave.git
cd keyweave
make install-release
```

Open Accessibility settings:

```sh
keyweave permissions
```

Add `~/.cargo/bin/keyweave` in **System Settings → Privacy & Security →
Accessibility**, enable it, then run:

```sh
keyweave restart
keyweave doctor
```

The default configuration is `~/.config/keyweave/config.toml`. A minimal
working configuration:

```toml
[hyper]
key = "caps_lock"
tap = "escape"
modifiers = ["command", "control", "option", "shift"]

[bindings]
"hyper+t" = { app = "Ghostty" }
"hyper+s" = { app = "Slack" }
"hyper+b" = { url = "https://github.com" }
"hyper+a" = { keys = "control+a" }
```

## Complete configuration example

This example demonstrates every action type and the main chord features:

```toml
[hyper]
key = "caps_lock"
tap = "escape"
modifiers = ["command", "control", "option", "shift"]

[bindings]
# Launch an application by name.
"hyper+t" = { app = "Ghostty" }

# Open a web URL or an application deep link.
"hyper+b" = { url = "https://github.com" }
"hyper+m" = { url = "mailto:" }

# Run through /bin/zsh -lc.
"hyper+r" = { command = "/Users/me/.local/bin/open-project" }

# Emit another keyboard shortcut.
"hyper+a" = { keys = "control+a" }

# Generic modifiers match either the left or right key.
"option+j" = { keys = "down" }

# Side-specific modifiers are also supported.
"left_option+h" = { keys = "left" }
"left_option+k" = { keys = "up" }
"left_option+l" = { keys = "right" }

# Keep a binding in the file without activating it.
"hyper+x" = { command = "say disabled", enabled = false }
```

Unknown fields, invalid names, duplicate normalized chords, and bindings with
more or fewer than one action are rejected by `keyweave validate`.

## Configuration reference

### `[hyper]`

The optional `[hyper]` table defines a dual-role key:

| Field | Meaning | Default |
|---|---|---|
| `key` | Physical key used as Hyper | `"caps_lock"` |
| `tap` | Key emitted when Hyper is pressed and released alone | `"escape"` |
| `modifiers` | Modifiers held while Hyper is down | `["command", "control", "option", "shift"]` |

For the standard Caps Lock setup:

```toml
[hyper]
key = "caps_lock"
tap = "escape"
modifiers = ["command", "control", "option", "shift"]
```

You can choose another supported key or a smaller modifier set:

```toml
[hyper]
key = "f19"
tap = "escape"
modifiers = ["command", "option"]
```

When `caps_lock` is the Hyper key, `keyweave` owns a macOS `hidutil` mapping
from Caps Lock to F18. F18 is therefore reserved and should not be configured
as a separate shortcut.

### `[bindings]`

Each entry maps a chord to an inline table. A binding must define exactly one
of `app`, `url`, `command`, or `keys`. `enabled` is optional and defaults to
`true`.

#### `app`

Launches or focuses an application using macOS `open -a`:

```toml
[bindings]
"hyper+t" = { app = "Ghostty" }
"hyper+s" = { app = "Slack" }
"hyper+f" = { app = "/Applications/Firefox.app" }
```

#### `url`

Opens a URL with its registered macOS handler. Web URLs and application deep
links both work:

```toml
[bindings]
"hyper+g" = { url = "https://github.com" }
"hyper+c" = { url = "raycast://extensions/raycast/system/open-camera" }
"hyper+m" = { url = "mailto:hello@example.com" }
```

The Raycast example only works if Raycast is installed and registered that URL
scheme; `keyweave` itself does not depend on Raycast.

#### `command`

Runs a command asynchronously with `/bin/zsh -lc`:

```toml
[bindings]
"hyper+n" = { command = "/Users/me/.local/bin/new-note" }
"hyper+p" = { command = "open /Users/me/code" }
"hyper+v" = { command = "pbpaste | /usr/bin/sed 's/^/→ /' | pbcopy" }
```

LaunchAgents receive a minimal `PATH`:

```text
/usr/bin:/bin:/usr/sbin:/sbin
```

Use absolute executable paths or set `PATH` inside your command/script.
Commands run with your user privileges, so treat the config as executable code.

#### `keys`

Emits another key or chord as synthetic keyboard events:

```toml
[bindings]
"hyper+a" = { keys = "control+a" }
"left_option+h" = { keys = "left" }
"left_option+j" = { keys = "down" }
"left_option+k" = { keys = "up" }
"left_option+l" = { keys = "right" }
```

An emitted `keys` action may contain the regular modifiers listed below, but it
cannot contain the virtual `hyper` modifier.

#### `enabled`

Set `enabled = false` to retain a binding without registering it:

```toml
[bindings]
"hyper+x" = { app = "Xcode", enabled = false }
```

Disabled entries are ignored before their chord and action are validated.

## Chord syntax

A chord consists of zero or more modifiers followed by one key, separated by
`+`:

```text
hyper+t
command+shift+p
left_option+h
f12
```

Names are case-insensitive, and `-` inside a name is normalized to `_` (for
example, `left-option+h` equals `left_option+h`). Modifier order does not
matter. As a result, `Command+Shift+P` and `shift+command+p` describe the same
chord and cannot both be configured.

A generic modifier such as `option` matches either physical Option key. A
side-specific binding such as `left_option+h` takes precedence over a generic
`option+h` binding when both could match.

### Supported modifiers

| Name | Aliases |
|---|---|
| `hyper` | — |
| `command` | `cmd` |
| `control` | `ctrl` |
| `option` | `alt` |
| `shift` | — |
| `fn` | `function` |
| `left_command` | `left_cmd` |
| `right_command` | `right_cmd` |
| `left_control` | `left_ctrl` |
| `right_control` | `right_ctrl` |
| `left_option` | `left_alt` |
| `right_option` | `right_alt` |
| `left_shift` | — |
| `right_shift` | — |

### Supported keys

| Group | Names |
|---|---|
| Letters | `a` through `z` |
| Numbers | `0` through `9` |
| Function keys | `f1` through `f20` |
| Common keys | `caps_lock`, `escape`, `enter`, `tab`, `space`, `delete`, `forward_delete` |
| Navigation | `left`, `right`, `up`, `down`, `home`, `end`, `page_up`, `page_down` |
| Punctuation | `minus`, `equal`, `left_bracket`, `right_bracket`, `backslash`, `semicolon`, `quote`, `comma`, `period`, `slash`, `grave` |

Key aliases:

| Alias | Canonical name |
|---|---|
| `esc` | `escape` |
| `return` | `enter` |
| `backspace` | `delete` |
| `left_arrow` | `left` |
| `right_arrow` | `right` |
| `up_arrow` | `up` |
| `down_arrow` | `down` |

Key names use ANSI keyboard positions. Letter shortcuts remain tied to their
physical ANSI key position when a different macOS input layout is active.

## Recipes

### Hyper navigation layer

```toml
[bindings]
"hyper+h" = { keys = "left" }
"hyper+j" = { keys = "down" }
"hyper+k" = { keys = "up" }
"hyper+l" = { keys = "right" }
"hyper+u" = { keys = "page_up" }
"hyper+d" = { keys = "page_down" }
```

### Application launcher

```toml
[bindings]
"hyper+t" = { app = "Ghostty" }
"hyper+s" = { app = "Slack" }
"hyper+f" = { app = "Firefox" }
"hyper+e" = { app = "Finder" }
```

### Websites and deep links

```toml
[bindings]
"hyper+g" = { url = "https://github.com" }
"hyper+i" = { url = "https://github.com/issues" }
"hyper+m" = { url = "mailto:" }
```

### Dotfiles scripts

Using scripts keeps complex behavior testable and portable:

```toml
[bindings]
"hyper+n" = { command = "/Users/me/dotfiles/bin/new-note" }
"hyper+w" = { command = "/Users/me/dotfiles/bin/open-workspace" }
```

### tmux prefix

This matches the Karabiner rule “Hyper+A → Control+A”:

```toml
[bindings]
"hyper+a" = { keys = "control+a" }
```

## Dotfiles and multiple Macs

Keep the canonical file in your dotfiles and symlink it into the default
location:

```sh
mkdir -p ~/.config/keyweave
ln -s ~/dotfiles/keyweave/config.toml ~/.config/keyweave/config.toml
keyweave restart
```

Alternatively, use a custom path:

```sh
keyweave --config ~/dotfiles/keyweave/config.toml validate
keyweave --config ~/dotfiles/keyweave/config.toml install
```

The LaunchAgent written by the second command remembers that custom path.
Machine-specific app names, executable paths, and URL handlers still need to
exist on each Mac. Run `keyweave doctor` after deploying.

## Commands

The global `--config <PATH>` option selects a non-default configuration for any
command.

| Command | Purpose |
|---|---|
| `keyweave init` | Create the default config if it does not exist |
| `keyweave init --force` | Replace the config with the generated default |
| `keyweave validate` | Parse and validate the selected config |
| `keyweave run` | Run the daemon in the foreground |
| `keyweave install` | Validate, stably sign, install, and start the LaunchAgent |
| `keyweave uninstall` | Stop and remove the LaunchAgent and owned HID mapping |
| `keyweave restart` | Restart the installed LaunchAgent |
| `keyweave status` | Show configuration and LaunchAgent status |
| `keyweave doctor` | Check config, signing, Accessibility, and LaunchAgent health |
| `keyweave permissions` | Open macOS Accessibility settings |
| `keyweave config-path` | Print the active config path |

Use `keyweave run` while developing to keep logs in the terminal. Stop the
installed agent first if necessary so two instances do not process the same
shortcut.

## Installation workflows

Install an optimized binary and configure the LaunchAgent:

```sh
make install-release
```

For a debug build:

```sh
make install
```

If you install directly with Cargo, finish by running `keyweave install`:

```sh
cargo install --path . --force
keyweave install
```

`cargo install` produces an ad-hoc signature. The second command replaces it
with a stable signature and refreshes the LaunchAgent.

## How it works

1. For the default Caps Lock setup, `keyweave` applies a narrowly scoped
   `hidutil` mapping from Caps Lock to F18.
2. A macOS event tap observes keyboard events globally.
3. Pressing the Hyper key holds the configured virtual modifiers. Releasing it
   without another key emits the configured tap key.
4. Matching chords dispatch their action on a worker thread so slow commands do
   not block keyboard input.
5. A per-user LaunchAgent starts the daemon at login and restarts it if needed.

The LaunchAgent is stored at
`~/Library/LaunchAgents/io.github.cesarferreira.keyweave.plist`.

## Troubleshooting

### Caps Lock acts like normal Caps Lock

```sh
keyweave restart
keyweave doctor
hidutil property --get UserKeyMapping
```

For the default setup, `hidutil` should report a Caps Lock → F18 mapping. Its
values are displayed as decimal HID usage codes. If `doctor` reports an
Accessibility problem, remove any stale `keyweave` entry from Accessibility,
add `~/.cargo/bin/keyweave` again, enable it, and restart.

### `doctor` says the binary is ad-hoc signed

```sh
keyweave install
keyweave doctor
```

Avoid finishing an update with only `cargo install`; it replaces the stably
signed executable.

### The LaunchAgent is not running

```sh
keyweave install
keyweave status
```

`Boot-out failed: 3: No such process` is harmless during installation when no
older agent was loaded. Installation continues by loading the new agent.

### A command works in Terminal but not from a binding

The LaunchAgent has a minimal `PATH`. Use absolute paths:

```toml
[bindings]
"hyper+r" = { command = "/opt/homebrew/bin/my-command" }
```

Or set the environment explicitly:

```toml
[bindings]
"hyper+r" = { command = "PATH=/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin my-command" }
```

### Two shortcuts are reported as duplicates

Chord names are normalized. Modifier order, aliases, case, and `-` versus `_`
do not create distinct shortcuts. Keep only one normalized form.

### Config changes are not active

The daemon reads the config when it starts:

```sh
keyweave validate
keyweave restart
```

### Another HID mapping is already installed

`keyweave` refuses to overwrite a `UserKeyMapping` it does not own. Remove or
disable the software that created the mapping, clear that mapping deliberately,
then run `keyweave restart`.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
cargo build
```

Run with a temporary config:

```sh
cargo run -- --config /path/to/config.toml validate
cargo run -- --config /path/to/config.toml run
```

## License

[MIT](LICENSE)
