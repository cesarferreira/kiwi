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
use serde::Serialize;

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

pub fn listen_event_tap(
    config_path: &Path,
    config: CompiledConfig,
    color: bool,
    json: bool,
) -> Result<()> {
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

            match observation_for(&handled, color, json) {
                Ok(Some(line)) => {
                    if let Err(error) = write_observation_line(&mut io::stdout().lock(), &line) {
                        if error.kind() != io::ErrorKind::BrokenPipe {
                            eprintln!("kiwi listen output failed: {error}");
                            callback_output_failed.store(true, Ordering::Relaxed);
                        }
                        CFRunLoop::get_current().stop();
                    }
                }
                Err(error) => {
                    eprintln!("kiwi listen output failed: {error}");
                    callback_output_failed.store(true, Ordering::Relaxed);
                    CFRunLoop::get_current().stop();
                }
                Ok(None) => {}
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

fn write_observation_line(writer: &mut impl Write, line: &str) -> io::Result<()> {
    writeln!(writer, "{line}")?;
    writer.flush()
}

fn observation_for(handled: &HandledEvent, color: bool, json: bool) -> Result<Option<String>> {
    let Some(chord) = handled.chord.as_ref() else {
        return Ok(None);
    };
    let action = match &handled.decision {
        Decision::Trigger(action) => Some(action),
        _ => None,
    };
    if json {
        Ok(Some(observation_json(&chord.to_string(), action)?))
    } else {
        Ok(Some(observation_line(&chord.to_string(), action, color)))
    }
}

#[derive(Serialize)]
struct ObservationOutput<'a> {
    schema_version: u8,
    shortcut: &'a str,
    matched: bool,
    #[serde(rename = "type")]
    kind: Option<&'static str>,
    action: Option<String>,
}

fn observation_json(shortcut: &str, action: Option<&Action>) -> serde_json::Result<String> {
    let (kind, action) = action.map_or((None, None), |action| {
        let (kind, value) = action.type_and_value();
        (Some(kind), Some(value))
    });
    serde_json::to_string(&ObservationOutput {
        schema_version: 1,
        shortcut,
        matched: kind.is_some(),
        kind,
        action,
    })
}

fn observation_line(shortcut: &str, action: Option<&Action>, color: bool) -> String {
    let shortcut = paint(shortcut, "36", color);
    let Some(action) = action else {
        return format!("{shortcut}  {}", paint("unmatched", "33", color));
    };

    let (kind, value) = action.type_and_value();
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
    use std::{
        io::{self, Write},
        sync::mpsc,
    };

    use super::{observation_for, observation_line};
    use crate::{
        config::{Action, AppAction, AppBehavior, Config},
        engine::{EventKind, Input},
        key::{Key, Modifier},
        macos::runtime::ReloadingEngine,
    };

    #[derive(Default)]
    struct RecordingWriter {
        written: Vec<u8>,
        flushes: usize,
        fail_flush: bool,
    }

    impl Write for RecordingWriter {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.written.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            self.flushes += 1;
            if self.fail_flush {
                Err(io::Error::other("flush failed"))
            } else {
                Ok(())
            }
        }
    }

    #[test]
    fn formats_every_matched_action_type() {
        let cases = [
            (
                "hyper+t",
                Action::App(AppAction {
                    target: "Ghostty".into(),
                    behavior: AppBehavior::Launch,
                }),
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
    fn formats_app_behaviors_in_text_and_ndjson() {
        for (behavior, suffix) in [
            (AppBehavior::Hide, "hide"),
            (AppBehavior::Cycle, "cycle"),
            (AppBehavior::NewWindow, "new window"),
        ] {
            let action = Action::App(AppAction {
                target: "Ghostty".into(),
                behavior,
            });
            assert_eq!(
                observation_line("hyper+t", Some(&action), false),
                format!("hyper+t  matched  app  Ghostty ({suffix})")
            );
            let json = super::observation_json("hyper+t", Some(&action)).unwrap();
            let json: serde_json::Value = serde_json::from_str(&json).unwrap();
            assert_eq!(json["type"], "app");
            assert_eq!(json["action"], format!("Ghostty ({suffix})"));
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
    fn formats_matched_observations_as_compact_json() {
        let output = super::observation_json(
            "hyper+p",
            Some(&Action::RunCommand("printf '\u{1b}[31m'".into())),
        )
        .unwrap();

        assert_eq!(
            output,
            concat!(
                "{\"schema_version\":1,\"shortcut\":\"hyper+p\",\"matched\":true,",
                "\"type\":\"command\",\"action\":\"printf '\\u001b[31m'\"}"
            )
        );
        assert!(!output.contains('\u{1b}'));
    }

    #[test]
    fn formats_unmatched_observations_as_compact_json_with_null_action_fields() {
        assert_eq!(
            super::observation_json("hyper+z", None).unwrap(),
            concat!(
                "{\"schema_version\":1,\"shortcut\":\"hyper+z\",\"matched\":false,",
                "\"type\":null,\"action\":null}"
            )
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
    fn writes_each_observation_and_flushes_immediately() {
        let mut writer = RecordingWriter::default();

        super::write_observation_line(&mut writer, "hyper+t  matched  app  Ghostty").unwrap();

        assert_eq!(writer.written, b"hyper+t  matched  app  Ghostty\n");
        assert_eq!(writer.flushes, 1);
    }

    #[test]
    fn treats_flush_failures_like_write_failures() {
        let mut writer = RecordingWriter {
            fail_flush: true,
            ..RecordingWriter::default()
        };

        let error = super::write_observation_line(&mut writer, "hyper+z  unmatched").unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Other);
        assert_eq!(writer.flushes, 1);
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
            assert_eq!(observation_for(&handled, false, false).unwrap(), None);
        }
    }
}
