use std::{
    ffi::c_void,
    path::Path,
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
        CGEventTapPlacement, CGEventType, CallbackResult, EventField,
    },
    event_source::{CGEventSource, CGEventSourceStateID},
};

use crate::{
    config::{Action, CompiledConfig},
    engine::Decision,
    key::{Chord, Modifier},
    reload::watch_config,
};

use super::{
    events::{EventDecoder, SYNTHETIC_EVENT_TAG},
    key_to_keycode,
    runtime::{ReloadNotice, ReloadingEngine},
};

const CAPS_TO_F18_MAPPING: &str = r#"{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x70000006D}]}"#;

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

pub fn run_event_tap(config_path: &Path, config: CompiledConfig) -> Result<()> {
    if !accessibility_is_trusted() {
        bail!(
            "Accessibility permission is required; add kiwi in System Settings > Privacy & Security > Accessibility"
        );
    }

    let config_receiver = watch_config(config_path)?;
    let remap_caps = config.hyper.key.as_str() == "caps_lock";
    if remap_caps {
        apply_caps_to_f18()?;
    }

    let engine = Arc::new(Mutex::new(ReloadingEngine::new(config, config_receiver)));
    let decoder = Arc::new(EventDecoder::new(remap_caps));
    let tap_port = Arc::new(AtomicPtr::<c_void>::new(std::ptr::null_mut()));
    let (action_sender, action_receiver) = mpsc::channel();
    thread::Builder::new()
        .name("kiwi-actions".into())
        .spawn(move || action_worker(action_receiver))
        .context("could not start action worker")?;

    let callback_engine = Arc::clone(&engine);
    let callback_decoder = Arc::clone(&decoder);
    let callback_port = Arc::clone(&tap_port);
    let callback_config_path = config_path.to_path_buf();
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

            let Some(input) = callback_decoder.input(event_type, event) else {
                return CallbackResult::Keep;
            };
            let handled = match callback_engine.lock() {
                Ok(mut engine) => engine.handle(input),
                Err(_) => return CallbackResult::Keep,
            };
            report_reload_notices(&callback_config_path, &handled.notices);
            apply_decision(handled.decision, event, &action_sender, remap_caps)
        },
    )
    .map_err(|()| {
        anyhow::anyhow!(
            "could not create the keyboard event tap; verify Accessibility permission for {}",
            std::env::current_exe()
                .map(|path| path.display().to_string())
                .unwrap_or_else(|_| "kiwi".into())
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
    println!("kiwi is running");
    CFRunLoop::run_current();
    Ok(())
}

pub(crate) fn report_reload_notices(config_path: &Path, notices: &[ReloadNotice]) {
    for notice in notices {
        match notice {
            ReloadNotice::Applied(binding_count) => eprintln!(
                "reloaded {} ({binding_count} enabled bindings)",
                config_path.display()
            ),
            ReloadNotice::HyperKeyChanged => {
                eprintln!("reload skipped: [hyper].key changed; run `kiwi restart` to apply it")
            }
        }
    }
}

fn apply_decision(
    decision: Decision,
    event: &CGEvent,
    actions: &mpsc::Sender<Action>,
    strip_caps_flag: bool,
) -> CallbackResult {
    match decision {
        Decision::Pass => {
            if strip_caps_flag
                && event
                    .get_flags()
                    .contains(CGEventFlags::CGEventFlagAlphaShift)
            {
                let replacement = event.clone();
                let mut flags = event.get_flags();
                flags.remove(CGEventFlags::CGEventFlagAlphaShift);
                replacement.set_flags(flags);
                CallbackResult::Replace(replacement)
            } else {
                CallbackResult::Keep
            }
        }
        Decision::Suppress => CallbackResult::Drop,
        Decision::Trigger(action) => {
            if let Err(error) = actions.send(action) {
                eprintln!("kiwi action worker stopped: {error}");
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

fn apply_caps_to_f18() -> Result<()> {
    match current_mapping_state()? {
        MappingState::Owned => return Ok(()),
        MappingState::Foreign => {
            bail!("another hidutil UserKeyMapping is already active; kiwi will not overwrite it");
        }
        MappingState::Empty => {}
    }
    let output = Command::new("/usr/bin/hidutil")
        .args(["property", "--set", CAPS_TO_F18_MAPPING])
        .output()
        .context("could not run hidutil")?;
    if !output.status.success() {
        bail!(
            "could not remap Caps Lock: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

pub fn remove_caps_remap() -> Result<()> {
    if current_mapping_state()? != MappingState::Owned {
        return Ok(());
    }
    let output = Command::new("/usr/bin/hidutil")
        .args(["property", "--set", r#"{"UserKeyMapping":[]}"#])
        .output()
        .context("could not run hidutil")?;
    if !output.status.success() {
        bail!(
            "could not restore Caps Lock: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(())
}

fn current_mapping_state() -> Result<MappingState> {
    let output = Command::new("/usr/bin/hidutil")
        .args(["property", "--get", "UserKeyMapping"])
        .output()
        .context("could not query hidutil")?;
    if !output.status.success() {
        bail!(
            "could not query key mappings: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(mapping_state(&String::from_utf8_lossy(&output.stdout)))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum MappingState {
    Empty,
    Owned,
    Foreign,
}

fn mapping_state(output: &str) -> MappingState {
    let output = output.trim();
    let compact: String = output
        .chars()
        .filter(|character| !character.is_whitespace())
        .collect();
    if output.is_empty() || output == "(null)" || compact == "()" {
        return MappingState::Empty;
    }
    let one_mapping = output.matches("HIDKeyboardModifierMappingSrc").count() == 1;
    let our_source = output.contains("30064771129") || output.contains("0x700000039");
    let our_destination = output.contains("30064771181") || output.contains("0x70000006D");
    if one_mapping && our_source && our_destination {
        MappingState::Owned
    } else {
        MappingState::Foreign
    }
}

fn action_worker(receiver: mpsc::Receiver<Action>) {
    for action in receiver {
        if let Err(error) = execute_action(&action) {
            eprintln!("kiwi action failed: {error:#}");
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

#[cfg(test)]
mod tests {
    use super::{CAPS_TO_F18_MAPPING, MappingState, mapping_state};

    #[test]
    fn uses_apples_caps_to_f18_hid_usage_mapping() {
        assert_eq!(
            CAPS_TO_F18_MAPPING,
            r#"{"UserKeyMapping":[{"HIDKeyboardModifierMappingSrc":0x700000039,"HIDKeyboardModifierMappingDst":0x70000006D}]}"#
        );
    }

    #[test]
    fn distinguishes_our_hid_mapping_from_foreign_mappings() {
        assert_eq!(mapping_state("(null)"), MappingState::Empty);
        assert_eq!(mapping_state("(\n)"), MappingState::Empty);
        assert_eq!(
            mapping_state(
                r#"(
                  {
                    HIDKeyboardModifierMappingDst = 30064771181;
                    HIDKeyboardModifierMappingSrc = 30064771129;
                  }
                )"#
            ),
            MappingState::Owned
        );
        assert_eq!(
            mapping_state(
                r#"(
                  {
                    HIDKeyboardModifierMappingDst = 30064771130;
                    HIDKeyboardModifierMappingSrc = 30064771129;
                  }
                )"#
            ),
            MappingState::Foreign
        );
    }
}
