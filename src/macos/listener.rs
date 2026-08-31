use std::{
    ffi::c_void,
    io::{self, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicPtr, Ordering},
    },
};

use anyhow::{Result, bail};
use core_foundation::{
    base::TCFType,
    mach_port::CFMachPortRef,
    runloop::{CFRunLoop, kCFRunLoopCommonModes},
};
use core_graphics::event::{
    CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement, CGEventType,
    CallbackResult, EventField,
};

use crate::{
    config::{Action, CompiledConfig},
    engine::Decision,
    reload::watch_config,
};

use super::{
    accessibility_is_trusted,
    daemon::report_reload_notices,
    events::{EventDecoder, SYNTHETIC_EVENT_TAG},
    runtime::{HandledEvent, ReloadingEngine},
};

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn CGEventTapEnable(tap: CFMachPortRef, enable: bool);
}

pub fn listen_event_tap(config_path: &Path, config: CompiledConfig, color: bool) -> Result<()> {
    if !accessibility_is_trusted() {
        bail!(
            "Accessibility permission is required for `kiwi listen`; add kiwi in System Settings > Privacy & Security > Accessibility"
        );
    }

    let config_receiver = watch_config(config_path)?;
    let remap_caps = config.hyper.key.as_str() == "caps_lock";
    let runtime = Arc::new(Mutex::new(ReloadingEngine::new(config, config_receiver)));
    let decoder = Arc::new(EventDecoder::new(remap_caps));
    let tap_port = Arc::new(AtomicPtr::<c_void>::new(std::ptr::null_mut()));
    let output_failed = Arc::new(AtomicBool::new(false));

    let callback_runtime = Arc::clone(&runtime);
    let callback_decoder = Arc::clone(&decoder);
    let callback_port = Arc::clone(&tap_port);
    let callback_output_failed = Arc::clone(&output_failed);
    let callback_config_path = config_path.to_path_buf();
    let event_tap = CGEventTap::new(
        CGEventTapLocation::HID,
        CGEventTapPlacement::HeadInsertEventTap,
        CGEventTapOptions::ListenOnly,
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
            let handled = match callback_runtime.lock() {
                Ok(mut runtime) => runtime.observe(input),
                Err(_) => return CallbackResult::Keep,
            };
            report_reload_notices(&callback_config_path, &handled.notices);

            if let Some(line) = observation_for(&handled, color) {
                if let Err(error) = writeln!(
                    io::stdout().lock(),
                    "{line}"
                ) {
                    if error.kind() != io::ErrorKind::BrokenPipe {
                        eprintln!("kiwi listen output failed: {error}");
                        callback_output_failed.store(true, Ordering::Relaxed);
                    }
                    CFRunLoop::get_current().stop();
                }
            }
            CallbackResult::Keep
        },
    )
    .map_err(|()| {
        anyhow::anyhow!(
            "could not create the listen-only keyboard event tap; verify Accessibility permission for {}",
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
        .map_err(|()| anyhow::anyhow!("could not create listener run loop source"))?;
    run_loop.add_source(&source, unsafe { kCFRunLoopCommonModes });
    event_tap.enable();
    eprintln!("kiwi is listening (press Control-C to stop)");
    CFRunLoop::run_current();

    if output_failed.load(Ordering::Relaxed) {
        bail!("listener stopped after an output error");
    }
    Ok(())
}

fn observation_for(handled: &HandledEvent, color: bool) -> Option<String> {
    let chord = handled.chord.as_ref()?;
    let action = match &handled.decision {
        Decision::Trigger(action) => Some(action),
        _ => None,
    };
    Some(observation_line(&chord.to_string(), action, color))
}

fn observation_line(shortcut: &str, action: Option<&Action>, color: bool) -> String {
    let shortcut = paint(shortcut, "36", color);
    let Some(action) = action else {
        return format!("{shortcut}  {}", paint("unmatched", "33", color));
    };

    let (kind, value) = match action {
        Action::LaunchApp(value) => ("app", value.clone()),
        Action::OpenUrl(value) => ("url", value.clone()),
        Action::RunCommand(value) => ("command", value.clone()),
        Action::SendKeys(value) => ("keys", value.to_string()),
    };
    format!(
        "{shortcut}  {}  {}  {value}",
        paint("matched", "32", color),
        paint(kind, "35", color)
    )
}

fn paint(value: &str, code: &str, color: bool) -> String {
    if color {
        format!("\u{1b}[{code}m{value}\u{1b}[0m")
    } else {
        value.into()
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::{observation_for, observation_line};
    use crate::{
        config::{Action, Config},
        engine::{EventKind, Input},
        key::{Key, Modifier},
        macos::runtime::ReloadingEngine,
    };

    #[test]
    fn formats_every_matched_action_type() {
        let cases = [
            (
                "hyper+t",
                Action::LaunchApp("Ghostty".into()),
                "hyper+t  matched  app  Ghostty",
            ),
            (
                "hyper+u",
                Action::OpenUrl("https://example.com".into()),
                "hyper+u  matched  url  https://example.com",
            ),
            (
                "hyper+p",
                Action::RunCommand("echo hi".into()),
                "hyper+p  matched  command  echo hi",
            ),
            (
                "hyper+a",
                Action::SendKeys("control+a".parse().unwrap()),
                "hyper+a  matched  keys  control+a",
            ),
        ];

        for (chord, action, expected) in cases {
            assert_eq!(observation_line(chord, Some(&action), false), expected);
        }
    }

    #[test]
    fn formats_an_unmatched_chord_without_an_action() {
        assert_eq!(
            observation_line("hyper+z", None, false),
            "hyper+z  unmatched"
        );
    }

    #[test]
    fn colors_each_semantic_field_when_requested() {
        assert_eq!(
            observation_line("hyper+p", Some(&Action::RunCommand("echo hi".into())), true,),
            concat!(
                "\u{1b}[36mhyper+p\u{1b}[0m  ",
                "\u{1b}[32mmatched\u{1b}[0m  ",
                "\u{1b}[35mcommand\u{1b}[0m  echo hi",
            )
        );
        assert_eq!(
            observation_line("hyper+z", None, true),
            "\u{1b}[36mhyper+z\u{1b}[0m  \u{1b}[33munmatched\u{1b}[0m"
        );
    }

    #[test]
    fn repeat_release_and_modifier_events_do_not_produce_observations() {
        let config = Config::from_toml("").unwrap().compile().unwrap();
        let (_sender, receiver) = mpsc::channel();
        let mut runtime = ReloadingEngine::new(config, receiver);
        let inputs = [
            Input::Modifier {
                modifier: Modifier::LeftOption,
                kind: EventKind::Down,
            },
            Input::Key {
                key: "h".parse::<Key>().unwrap(),
                kind: EventKind::Down,
                repeat: true,
            },
            Input::Key {
                key: "h".parse::<Key>().unwrap(),
                kind: EventKind::Up,
                repeat: false,
            },
        ];

        for input in inputs {
            let handled = runtime.observe(input);
            assert_eq!(observation_for(&handled, false), None);
        }
    }
}
