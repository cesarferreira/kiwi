//! Accessibility APIs identify and raise individual windows; Quartz supplies
//! current-Space visibility and front-to-back order without reading window titles.
use std::{collections::HashMap, ffi::c_void};

use anyhow::{Result, anyhow, bail};
use core_foundation::{
    array::CFArray,
    base::{CFType, CFTypeRef, TCFType},
    boolean::CFBoolean,
    dictionary::CFDictionary,
    number::CFNumber,
    string::CFString,
};
use core_graphics::{
    display::CGDisplay,
    event::CGEvent,
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::{CGPoint, CGRect, CGSize},
    window::{
        copy_window_info, kCGWindowListExcludeDesktopElements, kCGWindowListOptionOnScreenOnly,
    },
};
use objc2_app_kit::{NSApplicationActivationOptions, NSRunningApplication};

use crate::{config::WindowDirection, window_cycle::WindowCycle};

type AxRef = CFTypeRef;
#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    fn AXUIElementCreateApplication(pid: i32) -> AxRef;
    fn AXUIElementCreateSystemWide() -> AxRef;
    fn AXUIElementCopyAttributeValue(
        element: AxRef,
        attribute: CFTypeRef,
        value: *mut CFTypeRef,
    ) -> i32;
    fn AXUIElementSetAttributeValue(element: AxRef, attribute: CFTypeRef, value: CFTypeRef) -> i32;
    fn AXUIElementPerformAction(element: AxRef, action: CFTypeRef) -> i32;
    fn AXUIElementSetMessagingTimeout(element: AxRef, timeout: f32) -> i32;
    fn AXValueGetValue(value: CFTypeRef, kind: u32, output: *mut c_void) -> bool;
}

#[derive(Clone)]
struct Window {
    element: CFType,
    pid: i32,
}
impl PartialEq for Window {
    fn eq(&self, other: &Self) -> bool {
        self.element == other.element
    }
}
#[derive(Default)]
pub(crate) struct WindowController {
    cycle: WindowCycle<Window>,
    display: Option<CGRect>,
}
impl WindowController {
    pub fn reset(&mut self) {
        self.cycle.reset();
        self.display = None;
    }
    pub fn cycle(&mut self, direction: WindowDirection) -> Result<()> {
        // A system-wide AX timeout is the process default, including handles
        // returned from AXWindows and AXFocusedApplication. A timeout set only
        // on an application element would not cover its child window handles.
        let system = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateSystemWide()) };
        unsafe {
            AXUIElementSetMessagingTimeout(system.as_CFTypeRef(), 0.2);
        }
        let focused = focused_window();
        let display = match self.display {
            Some(display) => display,
            None => {
                let point = focused
                    .as_ref()
                    .and_then(bounds)
                    .map(|r| {
                        CGPoint::new(
                            r.origin.x + r.size.width / 2.0,
                            r.origin.y + r.size.height / 2.0,
                        )
                    })
                    .or_else(|| {
                        CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
                            .ok()
                            .and_then(|source| CGEvent::new(source).ok())
                            .map(|event| event.location())
                    })
                    .unwrap_or(CGPoint::new(0.0, 0.0));
                let displays = CGDisplay::active_displays()
                    .map_err(|code| anyhow!("could not enumerate displays: {code}"))?;
                let display = displays
                    .into_iter()
                    .map(|id| CGDisplay::new(id).bounds())
                    .find(|r| contains(*r, point))
                    .unwrap_or_else(|| CGDisplay::main().bounds());
                self.display = Some(display);
                display
            }
        };
        let visible = visible_windows(display)?;
        let focused =
            focused.and_then(|element| visible.iter().find(|w| w.element == element).cloned());
        // Prune inaccessible or closed candidates on the next gesture, retaining
        // the original order and deferring newly opened windows until release.
        let Some(window) = self.cycle.select(&visible, focused.as_ref(), direction) else {
            return Ok(());
        };
        let app = NSRunningApplication::runningApplicationWithProcessIdentifier(window.pid)
            .ok_or_else(|| anyhow!("selected window's application exited"))?;
        // Activating only this app plus setting AXMain and raising the selected
        // handle avoids the launcher's app-wide (all windows) activation behavior.
        #[allow(deprecated)]
        if !app.activateWithOptions(NSApplicationActivationOptions::ActivateIgnoringOtherApps) {
            bail!("could not activate selected window's application");
        }
        let main = CFString::new("AXMain");
        let focused_attribute = CFString::new("AXFocused");
        let focused_window_attribute = CFString::new("AXFocusedWindow");
        let application = application(window.pid);
        let raised = CFString::new("AXRaise");
        // SAFETY: Retained AX handles and CF strings/booleans live through calls.
        unsafe {
            AXUIElementSetAttributeValue(
                window.element.as_CFTypeRef(),
                main.as_CFTypeRef(),
                CFBoolean::true_value().as_CFTypeRef(),
            );
            // Apps differ in which focus attributes are settable. Attempt both
            // forms, then verify the actual focused window below.
            AXUIElementSetAttributeValue(
                window.element.as_CFTypeRef(),
                focused_attribute.as_CFTypeRef(),
                CFBoolean::true_value().as_CFTypeRef(),
            );
            AXUIElementSetAttributeValue(
                application.as_CFTypeRef(),
                focused_window_attribute.as_CFTypeRef(),
                window.element.as_CFTypeRef(),
            );
            let status =
                AXUIElementPerformAction(window.element.as_CFTypeRef(), raised.as_CFTypeRef());
            if status != 0 {
                bail!("could not raise selected window (Accessibility error {status})");
            }
        }
        // App activation can finish asynchronously; this bounded wait is on
        // the action worker, never on the event-tap callback.
        for _ in 0..10 {
            if focused_window().is_some_and(|focused| focused == window.element) {
                return Ok(());
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        bail!("selected window was raised but did not receive keyboard focus")
    }
}
fn contains(rect: CGRect, point: CGPoint) -> bool {
    point.x >= rect.origin.x
        && point.x < rect.origin.x + rect.size.width
        && point.y >= rect.origin.y
        && point.y < rect.origin.y + rect.size.height
}
fn attr(element: &CFType, name: &str) -> Option<CFType> {
    let name = CFString::new(name);
    let mut value = std::ptr::null();
    // SAFETY: Copy follows CF's create rule; failed/null outputs are not wrapped.
    unsafe {
        if AXUIElementCopyAttributeValue(element.as_CFTypeRef(), name.as_CFTypeRef(), &mut value)
            != 0
            || value.is_null()
        {
            return None;
        }
        Some(CFType::wrap_under_create_rule(value))
    }
}
fn application(pid: i32) -> CFType {
    // SAFETY: Creation returns a retained AX application element for any PID.
    unsafe {
        let app = CFType::wrap_under_create_rule(AXUIElementCreateApplication(pid));
        AXUIElementSetMessagingTimeout(app.as_CFTypeRef(), 0.2);
        app
    }
}
fn focused_window() -> Option<CFType> {
    // SAFETY: System-wide AX creation returns an owned non-null element.
    let system = unsafe { CFType::wrap_under_create_rule(AXUIElementCreateSystemWide()) };
    attr(&attr(&system, "AXFocusedApplication")?, "AXFocusedWindow")
}
fn bounds(window: &CFType) -> Option<CGRect> {
    let position = attr(window, "AXPosition")?;
    let size = attr(window, "AXSize")?;
    let mut point = CGPoint::new(0.0, 0.0);
    let mut dimensions = CGSize::new(0.0, 0.0);
    // SAFETY: AXValue kinds 1 and 2 require CGPoint and CGSize output respectively.
    unsafe {
        if !AXValueGetValue(
            position.as_CFTypeRef(),
            1,
            (&mut point as *mut CGPoint).cast(),
        ) || !AXValueGetValue(
            size.as_CFTypeRef(),
            2,
            (&mut dimensions as *mut CGSize).cast(),
        ) {
            return None;
        }
    }
    Some(CGRect::new(&point, &dimensions))
}
fn number(dict: &CFDictionary<CFString, CFType>, name: &str) -> Option<i64> {
    dict.find(CFString::new(name))?
        .downcast::<CFNumber>()?
        .to_i64()
}
fn visible_windows(display: CGRect) -> Result<Vec<Window>> {
    let info = copy_window_info(
        kCGWindowListOptionOnScreenOnly | kCGWindowListExcludeDesktopElements,
        0,
    )
    .ok_or_else(|| anyhow!("could not enumerate on-screen windows"))?;
    // AX has no public window-ID accessor. Resolve the established macOS bridge
    // at runtime so an OS removing it produces an action error, not a load crash.
    type WindowId = unsafe extern "C" fn(AxRef, *mut u32) -> i32;
    let symbol = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"_AXUIElementGetWindow".as_ptr()) };
    if symbol.is_null() {
        bail!("this macOS version does not expose the Accessibility window-ID bridge");
    }
    // SAFETY: This system symbol has the WindowId ABI; null is checked above.
    let window_id: WindowId = unsafe { std::mem::transmute(symbol) };
    let mut apps: HashMap<i32, Vec<(CFType, u32)>> = HashMap::new();
    let mut windows = Vec::new();
    for entry in info.iter() {
        // SAFETY: CGWindowListCopyWindowInfo returns an array of CF dictionaries.
        let dict: CFDictionary<CFString, CFType> =
            unsafe { CFDictionary::wrap_under_get_rule((*entry).cast()) };
        if number(&dict, "kCGWindowLayer") != Some(0) {
            continue;
        }
        let Some(pid) = number(&dict, "kCGWindowOwnerPID").and_then(|n| i32::try_from(n).ok())
        else {
            continue;
        };
        let Some(rect) = dict
            .find(CFString::new("kCGWindowBounds"))
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|d| CGRect::from_dict_representation(&d))
        else {
            continue;
        };
        let center = CGPoint::new(
            rect.origin.x + rect.size.width / 2.0,
            rect.origin.y + rect.size.height / 2.0,
        );
        if rect.size.width <= 0.0 || rect.size.height <= 0.0 || !contains(display, center) {
            continue;
        }
        let Some(id) = number(&dict, "kCGWindowNumber").and_then(|n| u32::try_from(n).ok()) else {
            continue;
        };
        let candidates = apps.entry(pid).or_insert_with(|| {
            let app = application(pid);
            attr(&app, "AXWindows")
                .and_then(|v| v.downcast::<CFArray>())
                .map(|array| {
                    array
                        .iter()
                        .filter_map(|v| {
                            // SAFETY: AXWindows returns live AX handles; retain each
                            // handle and query its ID with the resolved system ABI.
                            let element = unsafe { CFType::wrap_under_get_rule(*v) };
                            let mut id = 0;
                            if unsafe { window_id(element.as_CFTypeRef(), &mut id) } == 0 {
                                Some((element, id))
                            } else {
                                None
                            }
                        })
                        .collect()
                })
                .unwrap_or_default()
        });
        for (element, ax_id) in candidates.iter() {
            if *ax_id != id {
                continue;
            }
            if attr(element, "AXMinimized")
                .and_then(|v| v.downcast::<CFBoolean>())
                .is_some_and(|v| v.into())
            {
                continue;
            }
            let window = Window {
                element: element.clone(),
                pid,
            };
            if !windows.contains(&window) {
                windows.push(window);
            }
        }
    }
    Ok(windows)
}
