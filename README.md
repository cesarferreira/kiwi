<div align="center">
  <h1>kiwi 🥝</h1>

  <p><strong>Run portable macOS key mappings</strong></p>

  <p>
    <img alt="License" src="https://img.shields.io/badge/license-MIT-green">
    <img alt="Rust" src="https://img.shields.io/badge/rust-1.85%2B-orange">
    <img alt="Edition" src="https://img.shields.io/badge/edition-2024-blue">
    <a href="https://crates.io/crates/kiwi-keymapper"><img alt="crates.io" src="https://img.shields.io/crates/v/kiwi-keymapper.svg"></a>
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

`kiwi` replaces the common Karabiner-Elements + launcher combination with
one small native daemon. It can turn Caps Lock into Hyper when held and Escape
when tapped, launch applications, open URLs, run shell commands, and emit other
keyboard shortcuts.

- One declarative TOML configuration
- Global shortcuts available in every application
- Caps Lock → Hyper/Escape dual-role key
- App, URL, command, and key-sequence actions
- Generic and side-specific modifiers
- Automatic config reloads, including symlinked dotfiles
- A read-only live shortcut listener
- A LaunchAgent for automatic startup
- No Karabiner-Elements or Raycast dependency

<a id="install"></a>
## Install

The shortest path on macOS:

```sh
brew install cesarferreira/tap/kiwi
kiwi install
```

`kiwi install` signs the Homebrew-installed binary with your stable Apple
code-signing identity, creates the LaunchAgent, and starts it. Run it again
after each `brew upgrade kiwi`, because the new binary needs to be signed and
the LaunchAgent needs to point at its new Homebrew Cellar path.

### Requirements

- macOS
- Accessibility permission for the installed `kiwi` binary
- A stable Apple code-signing identity

Check that macOS can see a signing identity:

```sh
security find-identity -p codesigning -v
```

`kiwi install` prefers a **Developer ID Application** identity, then an
**Apple Development** identity. A stable signature matters because macOS
Accessibility permission is tied to the identity of the executable.

<a id="quickstart"></a>
## Build from source

To build from source instead, install Rust 1.85 or newer and ensure
`~/.cargo/bin` is on your `PATH`, then run:

```sh
git clone https://github.com/cesarferreira/kiwi.git
cd kiwi
make install-release
```

Open Accessibility settings:

```sh
kiwi permissions
```

Add `~/.cargo/bin/kiwi` in **System Settings → Privacy & Security →
Accessibility**, enable it, then run:

```sh
kiwi restart
kiwi doctor
```

The default configuration is `~/.config/kiwi/config.toml`. A minimal
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
more or fewer than one action are rejected by `kiwi validate`.

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

When `caps_lock` is the Hyper key, `kiwi` owns a macOS `hidutil` mapping
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
scheme; `kiwi` itself does not depend on Raycast.

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
mkdir -p ~/.config/kiwi
ln -s ~/dotfiles/kiwi/config.toml ~/.config/kiwi/config.toml
kiwi validate
```

Alternatively, use a custom path:

```sh
kiwi --config ~/dotfiles/kiwi/config.toml validate
kiwi --config ~/dotfiles/kiwi/config.toml install
```

The LaunchAgent written by the second command remembers that custom path.
Machine-specific app names, executable paths, and URL handlers still need to
exist on each Mac. Run `kiwi doctor` after deploying.

## Automatic reloads

The daemon watches the selected config and reloads valid edits automatically.
This also works when `~/.config/kiwi/config.toml` is a symlink into your
dotfiles. Changes are applied after the current key sequence finishes, so an
edit cannot split a held chord across two configurations.

If an edit is invalid, Kiwi reports the error in its log and keeps the last
valid configuration active. Fixing the file triggers another reload; no
restart is needed. Changes to bindings, the Hyper tap action, and its emitted
modifiers all reload live.

Changing `[hyper].key` still requires `kiwi restart` because the physical HID
mapping is established when the process starts. Kiwi rejects that part of a
live reload and prints the restart instruction instead of leaving the keyboard
in a mixed state.

## Inspect live shortcuts

Run the listener alongside the installed daemon to see how physical key presses
resolve without executing actions or changing events:

```sh
kiwi listen
```

Example output:

```text
hyper+t  matched  app  Ghostty
hyper+p  matched  command  /Users/me/.local/bin/open-project
hyper+z  unmatched
```

Interactive output is colored by chord, match state, and action type. Piped
output is plain text. Repeats, releases, modifier-only events, Kiwi-generated
synthetic keys, and a Hyper tap by itself are omitted. The listener uses the
same automatic reload behavior as the daemon.

For compact newline-delimited JSON, run `kiwi --format json listen`. Each
observation is one object on stdout:

```json
{"schema_version":1,"shortcut":"hyper+t","matched":true,"type":"app","action":"Ghostty"}
{"schema_version":1,"shortcut":"hyper+z","matched":false,"type":null,"action":null}
```

Config reload notices remain on stderr.

## Commands

Global options:

- `--config <PATH>` selects a non-default configuration.
- `--format text|json` selects output format and defaults to `text`. JSON is
  available for `list`, `listen`, and `status`.

| Command | Purpose |
|---|---|
| `kiwi init` | Create the default config if it does not exist |
| `kiwi init --force` | Replace the config with the generated default |
| `kiwi validate` | Parse and validate the selected config |
| `kiwi list` | Print enabled shortcuts as a colored table, or one JSON object |
| `kiwi run` | Run the daemon in the foreground |
| `kiwi listen` | Show resolved shortcuts without executing actions; JSON is NDJSON |
| `kiwi install` | Validate, stably sign, install, and start the LaunchAgent |
| `kiwi start` | Start an installed LaunchAgent |
| `kiwi stop` | Stop the LaunchAgent and restore Caps Lock without uninstalling |
| `kiwi uninstall` | Stop and remove the LaunchAgent and owned HID mapping |
| `kiwi restart` | Restart the installed LaunchAgent |
| `kiwi status` | Print a concise LaunchAgent status summary, or one JSON object |
| `kiwi doctor` | Check config, signing, Accessibility, and LaunchAgent health |
| `kiwi permissions` | Open macOS Accessibility settings |
| `kiwi config-path` | Print the active config path |

Use `kiwi run` while developing to keep logs in the terminal. Stop the
installed agent first if necessary so two daemons do not process the same
shortcut. `kiwi listen` is read-only and is designed to run alongside either
one.

JSON output uses schema version 1 and never includes terminal color escapes:

```sh
kiwi --format json list
kiwi --format json status
```

## Installation workflows

Install an optimized binary and configure the LaunchAgent:

```sh
make install-release
```

For a debug build:

```sh
make install
```

If you install directly with Cargo, finish by running `kiwi install`:

```sh
cargo install --path . --force
kiwi install
```

`cargo install` produces an ad-hoc signature. The second command replaces it
with a stable signature and refreshes the LaunchAgent.

## How it works

1. For the default Caps Lock setup, `kiwi` applies a narrowly scoped
   `hidutil` mapping from Caps Lock to F18.
2. A macOS event tap observes keyboard events globally.
3. Pressing the Hyper key holds the configured virtual modifiers. Releasing it
   without another key emits the configured tap key.
4. Matching chords dispatch their action on a worker thread so slow commands do
   not block keyboard input.
5. An event-driven watcher compiles config edits off the keyboard callback and
   swaps in only valid, compatible changes at an idle key boundary.
6. A per-user LaunchAgent starts the daemon at login and restarts it if needed.

The LaunchAgent is stored at
`~/Library/LaunchAgents/io.github.cesarferreira.kiwi.plist`.

## Performance

`kiwi` stays asleep between keyboard events and keeps the event-tap callback
small. A release build measured on a 14-core Apple M4 Pro with macOS 26.6.2,
Rust 1.98.0, and ten configured bindings produced:

| Metric | Result |
|---|---:|
| Release binary | 1.01 MiB |
| Idle daemon | <0.1% sampled CPU, 3.3 MiB physical footprint |
| CLI startup and config validation | ~4.0 ms |
| Config parse and compile | ~5.5 µs |
| Ordinary key down/up cycle | ~20 ns |
| Mapped Hyper shortcut cycle | ~68 ns |
| Unmapped Hyper shortcut cycle | ~61 ns |

The engine figures are medians from 11 samples with one to two million cycles
per sample. They measure in-process routing only; macOS event delivery and the
application, URL, or command launched by an action are outside that timing.
Run the same dependency-free benchmark with:

```sh
cargo bench --bench engine
```

Idle CPU was sampled for 15 seconds with Instruments Time Profiler (zero CPU
samples), physical memory with `footprint`, and CLI startup over 100 runs with
`hyperfine --shell=none`.

## Troubleshooting

### Caps Lock acts like normal Caps Lock

```sh
kiwi restart
kiwi doctor
hidutil property --get UserKeyMapping
```

For the default setup, `hidutil` should report a Caps Lock → F18 mapping. Its
values are displayed as decimal HID usage codes. If `doctor` reports an
Accessibility problem, remove any stale `kiwi` entry from Accessibility,
add `~/.cargo/bin/kiwi` again, enable it, and restart.

### `doctor` says the binary is ad-hoc signed

```sh
kiwi install
kiwi doctor
```

Avoid finishing an update with only `cargo install`; it replaces the stably
signed executable.

### The LaunchAgent is not running

```sh
kiwi install
kiwi status
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

Check the daemon log for a reload error, then validate the selected file:

```sh
kiwi validate
tail -n 20 ~/Library/Logs/kiwi.log
```

Invalid edits leave the last valid configuration running. Correct the file and
it reloads automatically. Only a `[hyper].key` change requires `kiwi restart`.

### Another HID mapping is already installed

`kiwi` refuses to overwrite a `UserKeyMapping` it does not own. Remove or
disable the software that created the mapping, clear that mapping deliberately,
then run `kiwi restart`.

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
