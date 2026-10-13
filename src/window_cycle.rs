//! A session keeps its initial order even when raising a window changes z-order.
use crate::config::WindowDirection;

pub struct WindowCycle<T> {
    order: Vec<T>,
    selected: Option<T>,
}
impl<T> Default for WindowCycle<T> {
    fn default() -> Self {
        Self {
            order: Vec::new(),
            selected: None,
        }
    }
}
impl<T: Clone + PartialEq> WindowCycle<T> {
    pub fn reset(&mut self) {
        self.order.clear();
        self.selected = None;
    }
    pub fn select(
        &mut self,
        visible: &[T],
        focused: Option<&T>,
        direction: WindowDirection,
    ) -> Option<T> {
        if self.order.is_empty() {
            self.order = visible.to_vec();
        }
        let anchor = self.selected.as_ref().or(focused);
        let index = anchor.and_then(|id| self.order.iter().position(|w| w == id));
        let len = self.order.len();
        for offset in 1..=len {
            let i = match (index, direction) {
                (Some(i), WindowDirection::Next) => (i + offset) % len,
                (Some(i), WindowDirection::Previous) => (i + len - offset % len) % len,
                (None, WindowDirection::Next) => offset - 1,
                (None, WindowDirection::Previous) => len - offset,
            };
            if visible.contains(&self.order[i]) {
                let window = self.order[i].clone();
                self.selected = Some(window.clone());
                return Some(window);
            }
        }
        None
    }
}
