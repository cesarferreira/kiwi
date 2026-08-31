use std::{
    collections::BTreeSet,
    ffi::c_void,
    process::Command,
    sync::{
        Arc, Mutex,
        atomic::{AtomicPtr, Ordering},
        mpsc,
    },
    thread,
};

use anyhow::{Context, Result, bail};
use core_foundation::{
    base::TCFType,
    mach_port::CFMachPortRef,
    runloop::{CFRunLoop, kCFRunLoopCommonModes},
};
use core_graphics::{
    event::{
        CGEvent, CGEventFlags, CGEventTap, CGEventTapLocation, CGEventTapOptions,
        CGEventTapPlacement, CGEventType, CallbackResult, EventField, KeyCode,
    },
    event_source::{CGEventSource, CGEventSourceStateID},
};

use crate::{
    config::{Action, CompiledConfig},
    engine::{Decision, Engine, EventKind, Input},
    key::{Chord, Modifier},
};

use super::{key_to_keycode, keycode_to_key, modifier_for_keycode};

const SYNTHETIC_EVENT_TAG: i64 = 0x4b_45_59_57_45_41_56_45;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXIsProcessTrusted() -> bool;
}

#[link(name = "CoreGraphics", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

pub fn accessibility_is_trusted() -> bool {
    // SAFETY: This parameterless system function is safe to query at any time.
    unsafe { AXIsProcessTrusted() }
}

pub fn run_event_tap(config: CompiledConfig) -> Result<()> {
    if !accessibility_is_trusted() {
        bail!(
            "Accessibility permission is required; add keyweave in System Settings > Privacy & Security > Accessibility"
        );
    }

    let engine = Arc::new(Mutex::new(Engine::new(config)));
    let modifier_tracker = Arc::new(Mutex::new(ModifierTracker::default()));
    let tap_port = Arc::new(AtomicPtr::<c_void>::new(std::ptr::null_mut()));
    let (action_sender, action_receiver) = mpsc::channel();
    thread::Builder::new()
        .name("keyweave-actions".into())
        .spawn(move || action_worker(action_receiver))
        .context("could not start action worker")?;

    let callback_engine = Arc::clone(&engine);
    let callback_tracker = Arc::clone(&modifier_tracker);
    let callback_port = Arc::clone(&tap_port);
    let event_tap = CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::Default,
        vec![
            CGEventType::KeyDown,
            CGEventType::KeyUp,
            CGEventType::FlagsChanged,
        ],
        move |_proxy, event_type, event| {
            if matches!(
                event_type,
                CGEventType::TapDisabledByTimeout | CGEventType::TapDisabledByUserInput
            ) {
                let port = callback_port.load(Ordering::Relaxed) as CFMachPortRef;
                if !port.is_null() {
                    // SAFETY: The pointer belongs to the live event tap held by this run loop.
                    unsafe { CGEventTapEnable(port, true) };
                }
                return CallbackResult::Keep;
            }
            if event.get_integer_value_field(EventField::EVENT_SOURCE_USER_DATA)
                == SYNTHETIC_EVENT_TAG
            {
                return CallbackResult::Keep;
            }

            let Some(input) = input_from_event(event_type, event, &callback_tracker) else {
                return CallbackResult::Keep;
            };
            let decision = match callback_engine.lock() {
                Ok(mut engine) => engine.handle(input),
                Err(_) => return CallbackResult::Keep,
            };
            apply_decision(decision, event, &action_sender)
        },
    )
    .map_err(|()| {
        anyhow::anyhow!(
            "could not create the keyboard event tap; verify Accessibility permission for {}",
            std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "keyweave".into())
        )
    })?;

    tap_port.store(
        event_tap.mach_port().as_concrete_TypeRef() as *mut c_void,
        Ordering::Relaxed,
    );
    let run_loop = CFRunLoop::get_current();
    let source = event_tap
        .mach_port()
        .create_runloop_source(0)
        .map_err(|()| anyhow::anyhow!("could not create event-tap run loop source"))?;
    run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
    event_tap.enable();
    println!("keyweave is running");
    CFRunLoop::run_current();
    Ok(())
}

fn input_from_event(
    event_type: CGEventType,
    event: &CGEvent,
    tracker: &Mutex<ModifierTracker>,
) -> Option<Input> {
    let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
    if matches!(event_type, CGEventType::FlagsChanged) {
        return match tracker.lock().ok()?.toggle(keycode)? {
            TrackedFlag::Modifier(modifier, kind) => Some(Input::Modifier { modifier, kind }),
            TrackedFlag::Hyper(kind) => Some(Input::Key {
                key: keycode_to_key(KeyCode::CAPS_LOCK)?,
                kind,
                repeat: false,
            }),
        };
    }

    let kind = match event_type {
        CGEventType::KeyDown => EventKind::Down,
        CGEventType::KeyUp => EventKind::Up,
        _ => return None,
    };
    Some(Input::Key {
        key: keycode_to_key(keycode)?,
        kind,
        repeat: event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0,
    })
}

fn apply_decision(
    decision: Decision,
    event: &CGEvent,
    actions: &mpsc::Sender<Action>,
) -> CallbackResult {
    match decision {
        Decision::Pass => CallbackResult::Keep,
        Decision::Suppress => CallbackResult::Drop,
        Decision::Trigger(action) => {
            if let Err(error) = actions.send(action) {
                eprintln!("keyweave action worker stopped: {error}");
            }
            CallbackResult::Drop
        }
        Decision::PassWithModifiers(modifiers) => {
            let replacement = event.clone();
            let mut flags = event.get_flags();
            flags.remove(CGEventFlags::CGEventFlagAlphaShift);
            flags.insert(flags_for(&modifiers));
            replacement.set_flags(flags);
            CallbackResult::Replace(replacement)
        }
    }
}

fn action_worker(receiver: mpsc::Receiver<Action>) {
    for action in receiver {
        if let Err(error) = execute_action(&action) {
            eprintln!("keyweave action failed: {error:#}");
        }
    }
}

fn execute_action(action: &Action) -> Result<()> {
    match action {
        Action::LaunchApp(app) => run_process(Command::new("/usr/bin/open").args(["-a", app])),
        Action::OpenUrl(url) => run_process(Command::new("/usr/bin/open").arg(url)),
        Action::RunCommand(command) => run_process(Command::new("/bin/zsh").args(["-lc", command])),
        Action::SendKeys(chord) => post_chord(chord),
    }
}

fn run_process(command: &mut Command) -> Result<()> {
    let status = command
        .status()
        .context("could not start configured action")?;
    if !status.success() {
        bail!("configured action exited with {status}");
    }
    Ok(())
}

fn post_chord(chord: &Chord) -> Result<()> {
    let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState)
        .map_err(|()| anyhow::anyhow!("could not create keyboard event source"))?;
    let keycode = key_to_keycode(&chord.key)?;
    let flags = flags_for(&chord.modifiers);
    for is_down in [true, false] {
        let event = CGEvent::new_keyboard_event(source.clone(), keycode, is_down)
            .map_err(|()| anyhow::anyhow!("could not create synthetic keyboard event"))?;
        event.set_flags(flags);
        event.set_integer_value_field(EventField::EVENT_SOURCE_USER_DATA, SYNTHETIC_EVENT_TAG);
        event.post(CGEventTapLocation::HID);
    }
    Ok(())
}

fn flags_for(modifiers: &[Modifier]) -> CGEventFlags {
    modifiers
        .iter()
        .fold(CGEventFlags::empty(), |mut flags, modifier| {
            match modifier {
                Modifier::Command | Modifier::LeftCommand | Modifier::RightCommand => {
                    flags.insert(CGEventFlags::CGEventFlagCommand);
                }
                Modifier::Control | Modifier::LeftControl | Modifier::RightControl => {
                    flags.insert(CGEventFlags::CGEventFlagControl);
                }
                Modifier::Option | Modifier::LeftOption | Modifier::RightOption => {
                    flags.insert(CGEventFlags::CGEventFlagAlternate);
                }
                Modifier::Shift | Modifier::LeftShift | Modifier::RightShift => {
                    flags.insert(CGEventFlags::CGEventFlagShift);
                }
                Modifier::Function => flags.insert(CGEventFlags::CGEventFlagSecondaryFn),
                Modifier::Hyper => {}
            }
            flags
        })
}

#[derive(Debug, Eq, PartialEq)]
enum TrackedFlag {
    Modifier(Modifier, EventKind),
    Hyper(EventKind),
}

#[derive(Default)]
struct ModifierTracker {
    pressed: BTreeSet<u16>,
}

impl ModifierTracker {
    fn toggle(&mut self, keycode: u16) -> Option<TrackedFlag> {
        let kind = if self.pressed.remove(&keycode) {
            EventKind::Up
        } else {
            self.pressed.insert(keycode);
            EventKind::Down
        };
        if keycode == KeyCode::CAPS_LOCK {
            Some(TrackedFlag::Hyper(kind))
        } else {
            modifier_for_keycode(keycode).map(|modifier| TrackedFlag::Modifier(modifier, kind))
        }
    }
}

#[cfg(test)]
mod tests {
    use core_graphics::event::KeyCode;

    use super::{ModifierTracker, TrackedFlag};
    use crate::{engine::EventKind, key::Modifier};

    #[test]
    fn flags_changed_events_toggle_physical_modifier_sides() {
        let mut tracker = ModifierTracker::default();

        assert_eq!(
            tracker.toggle(KeyCode::OPTION),
            Some(TrackedFlag::Modifier(Modifier::LeftOption, EventKind::Down))
        );
        assert_eq!(
            tracker.toggle(KeyCode::OPTION),
            Some(TrackedFlag::Modifier(Modifier::LeftOption, EventKind::Up))
        );
    }

    #[test]
    fn caps_lock_flags_changed_events_become_momentary_key_events() {
        let mut tracker = ModifierTracker::default();

        assert_eq!(
            tracker.toggle(KeyCode::CAPS_LOCK),
            Some(TrackedFlag::Hyper(EventKind::Down))
        );
        assert_eq!(
            tracker.toggle(KeyCode::CAPS_LOCK),
            Some(TrackedFlag::Hyper(EventKind::Up))
        );
    }
}
