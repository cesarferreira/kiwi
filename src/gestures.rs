//! Recognition of phased, continuous horizontal scrolls (trackpad two-finger swipes).
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
    /// Point deltas normalized to physical finger motion: right is positive.
    pub x: i64,
    pub y: i64,
    pub phase: ScrollPhase,
    pub momentum: bool,
    pub continuous: bool,
}

#[derive(Default)]
pub(crate) struct SwipeRecognizer {
    armed: bool,
    suppress: bool,
    fired: bool,
    x: i64,
    y: i64,
    left: bool,
    right: bool,
}
pub(crate) struct SwipeOutcome {
    pub gesture: Option<&'static str>,
    pub suppress: bool,
}
impl SwipeRecognizer {
    pub fn release_hyper(&mut self) {
        self.armed = false;
    }
    pub fn handle(&mut self, scroll: Scroll, hyper: bool, left: bool, right: bool) -> SwipeOutcome {
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
                left,
                right,
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
            self.y = self.y.saturating_add(scroll.y.saturating_abs());
            // Lock out vertical gestures so changing direction cannot become a swipe.
            if self.y >= 30 && self.y > self.x.saturating_abs() {
                self.armed = false;
            } else if self.x.saturating_abs() > self.y.saturating_mul(2) {
                let bound = if self.x > 0 {
                    self.right && right
                } else {
                    self.left && left
                };
                self.suppress |= bound;
                if bound && self.x.saturating_abs() >= 30 {
                    gesture = Some(if self.x > 0 {
                        "swipe_right"
                    } else {
                        "swipe_left"
                    });
                    self.fired = true;
                }
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
