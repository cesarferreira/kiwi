//! Recognition of phased, continuous scrolls (trackpad two-finger swipes).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScrollPhase {
    None,
    Began,
    Changed,
    Ended,
    Cancelled,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Scroll {
    /// Point deltas normalized to physical finger motion: right/down are positive.
    pub x: i64,
    pub y: i64,
    pub phase: ScrollPhase,
    pub momentum: bool,
    pub continuous: bool,
}

#[derive(Clone, Copy, Default)]
pub(crate) struct GestureBindings {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
}

#[derive(Default)]
pub(crate) struct SwipeRecognizer {
    armed: bool,
    suppress: bool,
    fired: bool,
    x: i64,
    y: i64,
    travel_x: i64,
    travel_y: i64,
    bindings: GestureBindings,
}
pub(crate) struct SwipeOutcome {
    pub gesture: Option<&'static str>,
    pub suppress: bool,
}
impl SwipeRecognizer {
    pub fn release_hyper(&mut self) {
        self.armed = false;
    }
    pub fn handle(
        &mut self,
        scroll: Scroll,
        hyper: bool,
        bindings: GestureBindings,
    ) -> SwipeOutcome {
        if scroll.momentum {
            return SwipeOutcome {
                gesture: None,
                suppress: self.suppress,
            };
        }
        if scroll.phase == ScrollPhase::None || !scroll.continuous {
            *self = Self::default();
            return SwipeOutcome {
                gesture: None,
                suppress: false,
            };
        }
        if scroll.phase == ScrollPhase::Began {
            *self = Self {
                armed: hyper && scroll.continuous,
                bindings,
                ..Self::default()
            };
        }
        if !hyper {
            self.armed = false;
        }
        let mut gesture = None;
        if self.armed
            && !self.fired
            && matches!(scroll.phase, ScrollPhase::Began | ScrollPhase::Changed)
        {
            self.x = self.x.saturating_add(scroll.x);
            self.y = self.y.saturating_add(scroll.y);
            self.travel_x = self.travel_x.saturating_add(scroll.x.saturating_abs());
            self.travel_y = self.travel_y.saturating_add(scroll.y.saturating_abs());
            let candidate = if self.x.saturating_abs() > self.travel_y.saturating_mul(2) {
                if self.x > 0 {
                    Some((
                        "swipe_right",
                        self.x.saturating_abs(),
                        self.bindings.right && bindings.right,
                    ))
                } else {
                    Some((
                        "swipe_left",
                        self.x.saturating_abs(),
                        self.bindings.left && bindings.left,
                    ))
                }
            } else if self.y.saturating_abs() > self.travel_x.saturating_mul(2) {
                if self.y > 0 {
                    Some((
                        "swipe_down",
                        self.y.saturating_abs(),
                        self.bindings.down && bindings.down,
                    ))
                } else {
                    Some((
                        "swipe_up",
                        self.y.saturating_abs(),
                        self.bindings.up && bindings.up,
                    ))
                }
            } else {
                None
            };
            if let Some((name, distance, bound)) = candidate {
                self.suppress |= bound;
                if distance >= 30 {
                    // An intentional unbound gesture must not turn into another
                    // direction later in the same finger contact.
                    self.armed = false;
                    if bound {
                        gesture = Some(name);
                        self.fired = true;
                    } else {
                        self.suppress = false;
                    }
                }
            } else if self.travel_x >= 30 && self.travel_y >= 30 {
                // Initial one-axis jitter can look bound before the contact's
                // intent is known. Let substantial diagonal scrolling through.
                self.armed = false;
                self.suppress = false;
            }
        }
        if matches!(scroll.phase, ScrollPhase::Ended | ScrollPhase::Cancelled) {
            self.armed = false;
        }
        SwipeOutcome {
            gesture,
            suppress: self.suppress,
        }
    }
}
