# Hot Reload and Live Listener Design

## Summary

Kiwi will reload valid configuration changes without restarting its process and
will add a `kiwi listen` command that observes shortcut resolution alongside an
already-running daemon. Both features reuse the same config compiler and
keyboard engine so their results cannot drift from normal shortcut behavior.

The daemon remains small and event-driven. Filesystem changes use native macOS
FSEvents through `notify` 8.2.0; there is no polling loop, IPC server, or
persistent state.

## Goals

- Apply valid config edits before the next shortcut without requiring
  `kiwi restart`.
- Keep the last valid config active when a new file is invalid, missing, or
  temporarily incomplete during an editor save.
- Observe normalized chords and their resolved actions without stopping or
  interfering with the installed daemon.
- Support configs symlinked from dotfiles as well as regular files.
- Preserve Kiwi's near-zero idle CPU behavior and keep file parsing off the
  event-tap callback.

## Non-goals

- Watching included files or directories; Kiwi still has one config file.
- Remote control, an IPC protocol, or triggering actions from `listen`.
- Recording raw keystrokes to disk.
- A persistent event history or menu-bar interface.
- Changing shortcut syntax or action semantics.

## User Experience

### Hot reload

`kiwi run` and the installed LaunchAgent watch the selected config
automatically. No new flag is required.

After a valid edit, Kiwi writes a concise diagnostic to its existing output:

```text
reloaded /Users/cesarferreira/.config/kiwi/config.toml (9 shortcuts)
```

After an invalid edit, Kiwi retains the current config and reports:

```text
reload failed: invalid binding `hyper+p`: `command` cannot be empty; keeping previous config
```

Repeated filesystem events for the same editor save produce one reload result.
A later valid edit clears the failure naturally by replacing the active config.

### Listener

`kiwi listen` runs until interrupted with Ctrl-C. It opens a read-only event tap
at the head of the HID event chain, allowing it to observe input before the
installed daemon suppresses or rewrites that input. It does not stop, restart,
or otherwise control the daemon.

Startup and reload diagnostics go to stderr. Observations go to stdout, making
the command useful in a pipeline. Example terminal output:

```text
listening — press Ctrl-C to stop
hyper+p          matched    command  bluepods connect "César’s AirPods Pro #3"
hyper+z          unmatched
left_option+h    matched    keys     left
```

The shortcut is cyan, `matched` is green, `unmatched` is yellow, and the action
type is magenta when stdout is a terminal. Piped output contains no ANSI color.

The listener prints one line for each non-repeat key-down event. Modifier-only
events and key-up events are silent, except that tapping the configured Hyper
key prints its configured tap action. Key repeat does not flood the output.

The listener loads and watches the same selected `--config` path independently,
so its resolution stays in sync as both processes hot-reload.

## Architecture

### Config watcher

A new watcher component owns a `notify::RecommendedWatcher`. It watches the
logical config parent and, when the config is a symlink, the resolved target
parent. Duplicate directories are watched once. Watching the parent rather than
only the file preserves notifications across atomic rename-based editor saves.

Relevant create, modify, remove, and rename events enter a worker channel. The
worker waits for a 100 ms quiet period, drains the rest of that event burst,
then reads and compiles the logical config path. Parsing and filesystem work
never run inside the keyboard callback.

Successful compilation sends a `CompiledConfig` over a channel to the event-tap
runtime. Failed compilation logs one error and sends nothing, leaving the last
valid config untouched. If the runtime receiver has closed, the watcher exits
and releases its FSEvent resources.

Watcher setup happens before the event tap starts. Failure to create the watcher
or register either directory is fatal and includes the affected path in its
error context; Kiwi does not silently run without promised hot reload.

### Runtime config replacement

The event-tap callback drains completed configs without blocking and keeps only
the newest pending config. The engine exposes two small lifecycle operations:

- `is_idle`: true when no Hyper key, physical modifier, consumed key, or
  rewritten key is held.
- `replace_config`: swaps only the compiled config.

A pending config is installed immediately before handling an event when the
engine is idle. If a reload arrives during a chord, it remains pending until
the chord returns the engine to idle, then installs after that event. This
prevents config changes from splitting a key-down/key-up pair or leaving Hyper
state latched. The successful reload message is printed only when installation
actually occurs.

### Event-tap modes

The macOS daemon exposes two explicit entry points backed by shared event
decoding:

- Normal mode uses `CGEventTapOptions::Default`, owns the Caps-to-F18 mapping,
  executes actions on the worker thread, and applies engine decisions.
- Listen mode uses `CGEventTapOptions::ListenOnly`, never changes the HID
  mapping, never starts an action worker, and always returns
  `CallbackResult::Keep`.

Both modes use `Engine` for modifier, Hyper, and binding resolution. A preview
operation derives the normalized chord for a key-down from the engine's current
state before the event is handled. The listener combines that chord with the
resulting `Decision` to render `matched` or `unmatched`. Normal mode does not
perform preview work, so the listener adds no cost to the daemon's hot path.

The listener maps either physical Caps Lock or Kiwi's existing F18 HID mapping
to the configured Hyper key. This lets it work whether the installed daemon is
running or not, without taking ownership of that mapping.

## Error Handling

- Missing Accessibility permission uses the same actionable error as normal
  foreground mode.
- Event-tap creation failures identify `kiwi listen` when in listen mode.
- Watcher startup errors are fatal; post-start filesystem and config errors are
  recoverable and retain the active config.
- A burst of identical notification errors is collapsed by the debounce cycle.
- Poisoned runtime locks pass the physical event unchanged and emit no action,
  matching the daemon's current fail-safe behavior.
- Listener output failures such as a closed pipe end the listener cleanly rather
  than affecting the installed daemon.

## Dependencies

Add `notify = "8.2.0"`. This is the stable release line, supports Rust 1.77 and
therefore Kiwi's Rust 1.85 floor, and uses macOS FSEvents by default. Kiwi will
use only `RecommendedWatcher`, `Watcher`, `RecursiveMode::NonRecursive`, event
kinds, and paths from its public API.

No asynchronous runtime, debounce helper crate, or color dependency is needed.

## Testing

Tests are written before production changes and cover:

- A valid watcher event compiles and delivers a replacement config.
- Invalid config retains the last valid engine behavior.
- A symlinked dotfiles config reloads when its resolved target changes.
- Event bursts are debounced into one reload result with a bounded timeout.
- Config replacement waits for an active chord to finish and applies before the
  next chord.
- Changing the Hyper key cannot strand the previous Hyper state.
- Listener formatting distinguishes matched and unmatched chords, formats all
  action types, suppresses repeats and modifier-only events, and removes ANSI
  color when piped.
- CLI help exposes `kiwi listen` and `--config` selects the watched file.
- Existing engine, daemon, config, lifecycle, and performance tests remain
  green.

The final live check runs `kiwi listen` alongside the installed LaunchAgent,
confirms a configured shortcut is both printed and executed, edits the config,
and confirms both processes resolve the new binding without either restarting.

## Performance Constraints

The watcher must be event-driven and consume no measurable CPU while the config
is unchanged. Normal event routing must stay within ordinary benchmark noise;
only listen mode constructs preview chords and formats output. The release
binary size and idle footprint will be remeasured and the README Performance
section updated if either published figure changes materially.
