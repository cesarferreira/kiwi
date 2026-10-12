use std::{collections::BTreeSet, sync::Mutex};

use core_graphics::event::{CGEvent, CGEventType, EventField, KeyCode};

use crate::gestures::{Scroll, ScrollPhase};
use foreign_types::ForeignType;
use objc2_app_kit::NSEvent;

use crate::engine::{EventKind, Input};

use super::{keycode_to_key, modifier_for_keycode};

pub(crate) const SYNTHETIC_EVENT_TAG: i64 = 0x4b_45_59_57_45_41_56_45;

pub(crate) struct EventDecoder {
    modifier_tracker: Mutex<ModifierTracker>,
    remap_caps: bool,
}

impl EventDecoder {
    pub fn new(remap_caps: bool) -> Self {
        Self {
            modifier_tracker: Mutex::new(ModifierTracker::default()),
            remap_caps,
        }
    }

    pub fn input(&self, event_type: CGEventType, event: &CGEvent) -> Option<Input> {
        if matches!(event_type, CGEventType::ScrollWheel) {
            // Quartz's public phase fields (99/123) are missing from core-graphics's
            // EventField constants. NSEvent exposes the user's natural-scroll flag.
            // SAFETY: Both wrappers represent the same live CGEventRef.
            let inverted = objc2::rc::autoreleasepool(|_| {
                NSEvent::eventWithCGEvent(unsafe {
                    &*event.as_ptr().cast::<objc2_core_graphics::CGEvent>()
                })
                .map(|ns| ns.isDirectionInvertedFromDevice())
            })?;
            let x =
                event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2);
            let raw_phase = event.get_integer_value_field(99);
            let phase = if raw_phase & 1 != 0 {
                ScrollPhase::Began
            } else if raw_phase & 4 != 0 {
                ScrollPhase::Ended
            } else if raw_phase & 8 != 0 {
                ScrollPhase::Cancelled
            } else if raw_phase & 2 != 0 {
                ScrollPhase::Changed
            } else {
                ScrollPhase::None
            };
            return Some(Input::Scroll(Scroll {
                x: if inverted { x } else { x.saturating_neg() },
                y: event.get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1),
                phase,
                momentum: event.get_integer_value_field(123) != 0,
                continuous: event
                    .get_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS)
                    != 0,
            }));
        }
        let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE) as u16;
        if matches!(event_type, CGEventType::FlagsChanged) {
            let (modifier, kind) = self.modifier_tracker.lock().ok()?.toggle(keycode)?;
            return Some(Input::Modifier { modifier, kind });
        }

        keyboard_input(
            event_type,
            keycode,
            event.get_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT) != 0,
            self.remap_caps,
        )
    }
}

fn keyboard_input(
    event_type: CGEventType,
    keycode: u16,
    repeat: bool,
    remap_caps: bool,
) -> Option<Input> {
    let kind = match event_type {
        CGEventType::KeyDown => EventKind::Down,
        CGEventType::KeyUp => EventKind::Up,
        _ => return None,
    };
    let key = if remap_caps && keycode == KeyCode::F18 {
        keycode_to_key(KeyCode::CAPS_LOCK)?
    } else {
        keycode_to_key(keycode)?
    };
    Some(Input::Key { key, kind, repeat })
}

#[derive(Default)]
struct ModifierTracker {
    pressed: BTreeSet<u16>,
}

impl ModifierTracker {
    fn toggle(&mut self, keycode: u16) -> Option<(crate::key::Modifier, EventKind)> {
        let kind = if self.pressed.remove(&keycode) {
            EventKind::Up
        } else {
            self.pressed.insert(keycode);
            EventKind::Down
        };
        modifier_for_keycode(keycode).map(|modifier| (modifier, kind))
    }
}

#[cfg(test)]
mod tests {
    use core_graphics::{
        event::{CGEvent, CGEventType, EventField, KeyCode},
        event_source::{CGEventSource, CGEventSourceStateID},
    };

    use super::EventDecoder;
    use crate::{
        engine::{EventKind, Input},
        key::Modifier,
    };

    fn keyboard_event(keycode: u16, is_down: bool, repeat: bool) -> CGEvent {
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).unwrap();
        let event = CGEvent::new_keyboard_event(source, keycode, is_down).unwrap();
        event.set_integer_value_field(EventField::KEYBOARD_EVENT_AUTOREPEAT, i64::from(repeat));
        event
    }

    #[test]
    fn decodes_quartz_scroll_phases_and_momentum_without_treating_them_as_keys() {
        use crate::gestures::{Scroll, ScrollPhase};
        let decoder = EventDecoder::new(false);
        let source = CGEventSource::new(CGEventSourceStateID::HIDSystemState).unwrap();
        let event = CGEvent::new(source).unwrap();
        event.set_type(CGEventType::ScrollWheel);
        event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_IS_CONTINUOUS, 1);
        event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_1, 5);
        event.set_integer_value_field(EventField::SCROLL_WHEEL_EVENT_POINT_DELTA_AXIS_2, 40);
        // Synthetic Quartz scrolls have no device-inversion flag, so physical
        // rightward motion is the negative of their horizontal scroll delta.
        for (raw, phase) in [
            (1, ScrollPhase::Began),
            (2, ScrollPhase::Changed),
            (4, ScrollPhase::Ended),
            (8, ScrollPhase::Cancelled),
            (128, ScrollPhase::None),
        ] {
            event.set_integer_value_field(99, raw);
            assert_eq!(
                decoder.input(CGEventType::ScrollWheel, &event),
                Some(Input::Scroll(Scroll {
                    x: -40,
                    y: 5,
                    phase,
                    momentum: false,
                    continuous: true,
                }))
            );
        }
        event.set_integer_value_field(123, 2);
        let Some(Input::Scroll(scroll)) = decoder.input(CGEventType::ScrollWheel, &event) else {
            panic!("expected scroll");
        };
        assert!(scroll.momentum);
    }

    #[test]
    fn flags_changed_events_toggle_physical_modifier_sides() {
        let decoder = EventDecoder::new(false);
        let event = keyboard_event(KeyCode::OPTION, true, false);

        assert_eq!(
            decoder.input(CGEventType::FlagsChanged, &event),
            Some(Input::Modifier {
                modifier: Modifier::LeftOption,
                kind: EventKind::Down,
            })
        );
        assert_eq!(
            decoder.input(CGEventType::FlagsChanged, &event),
            Some(Input::Modifier {
                modifier: Modifier::LeftOption,
                kind: EventKind::Up,
            })
        );
    }

    #[test]
    fn remapped_f18_decodes_as_caps_with_real_down_and_up_events() {
        let decoder = EventDecoder::new(true);

        assert_eq!(
            decoder.input(
                CGEventType::KeyDown,
                &keyboard_event(KeyCode::F18, true, false)
            ),
            Some(Input::Key {
                key: "caps_lock".parse().unwrap(),
                kind: EventKind::Down,
                repeat: false,
            })
        );
        assert_eq!(
            decoder.input(
                CGEventType::KeyUp,
                &keyboard_event(KeyCode::F18, false, false)
            ),
            Some(Input::Key {
                key: "caps_lock".parse().unwrap(),
                kind: EventKind::Up,
                repeat: false,
            })
        );
    }

    #[test]
    fn decoder_preserves_repeat_state() {
        let decoder = EventDecoder::new(false);

        assert_eq!(
            decoder.input(
                CGEventType::KeyDown,
                &keyboard_event(KeyCode::ANSI_A, true, true)
            ),
            Some(Input::Key {
                key: "a".parse().unwrap(),
                kind: EventKind::Down,
                repeat: true,
            })
        );
    }
}
