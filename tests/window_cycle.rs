use kiwi_keymapper::{config::WindowDirection, window_cycle::WindowCycle};
#[test]
fn cycle_order_survives_focus_reordering_wraps_and_reverses() {
    let mut c = WindowCycle::default();
    assert_eq!(
        c.select(&[1, 2, 3], Some(&1), WindowDirection::Next),
        Some(2)
    );
    assert_eq!(
        c.select(&[2, 1, 3], Some(&2), WindowDirection::Next),
        Some(3)
    );
    assert_eq!(
        c.select(&[3, 2, 1], Some(&3), WindowDirection::Next),
        Some(1)
    );
    assert_eq!(
        c.select(&[1, 3, 2], Some(&1), WindowDirection::Previous),
        Some(3)
    );
    c.reset();
    assert_eq!(
        c.select(&[3, 1, 2], Some(&3), WindowDirection::Next),
        Some(1)
    );
}
#[test]
fn skips_closed_windows_defers_new_windows_until_reset_and_handles_empty() {
    let mut c = WindowCycle::default();
    assert_eq!(
        c.select(&[1, 2, 3], Some(&1), WindowDirection::Next),
        Some(2)
    );
    assert_eq!(
        c.select(&[4, 3, 1], Some(&1), WindowDirection::Next),
        Some(3)
    );
    assert_eq!(c.select(&[], None, WindowDirection::Next), None);
    c.reset();
    assert_eq!(c.select(&[5], None, WindowDirection::Previous), Some(5));
}
