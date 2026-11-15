//! OS-specific bits: focus handling and "what is under the pointer".

/// Handle of the window that had focus, to paste back into it.
#[derive(Clone, Copy, Default)]
pub struct Focus(pub isize);

#[cfg(windows)]
mod imp {
    use super::Focus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, SetForegroundWindow};

    pub fn foreground() -> Focus {
        Focus(unsafe { GetForegroundWindow() } as isize)
    }

    /// Gives focus back to `f`; false when Windows refuses, so we never paste into the wrong window.
    pub fn restore(f: Focus) -> bool {
        if f.0 == 0 {
            return false;
        }
        unsafe {
            if GetForegroundWindow() as isize != f.0 {
                SetForegroundWindow(f.0 as _);
            }
            GetForegroundWindow() as isize == f.0
        }
    }

    /// Accessible element under a screen point: role and name, and the nearest named ancestors.
    pub fn element_at(x: i32, y: i32) -> Option<String> {
        use uiautomation::types::Point;
        let ua = uiautomation::UIAutomation::new().ok()?;
        let el = ua.element_from_point(Point::new(x, y)).ok()?;
        let describe = |e: &uiautomation::UIElement| -> Option<String> {
            let role = e.get_localized_control_type().unwrap_or_default();
            let name = e.get_name().unwrap_or_default();
            let name: String = name.split_whitespace().collect::<Vec<_>>().join(" ");
            if name.is_empty() {
                return None;
            }
            let name = if name.chars().count() > 120 { format!("{}…", name.chars().take(120).collect::<String>()) } else { name };
            Some(if role.is_empty() { format!("« {name} »") } else { format!("{role} « {name} »") })
        };
        let mut parts = vec![];
        if let Some(d) = describe(&el) {
            parts.push(d);
        } else {
            let role = el.get_localized_control_type().unwrap_or_default();
            if !role.is_empty() {
                parts.push(role);
            }
        }
        let walker = ua.get_control_view_walker().ok()?;
        let mut cur = el;
        for _ in 0..6 {
            let Ok(p) = walker.get_parent(&cur) else { break };
            if let Some(d) = describe(&p) {
                parts.push(format!("dans {d}"));
                if parts.len() >= 3 {
                    break;
                }
            }
            cur = p;
        }
        if parts.is_empty() { None } else { Some(parts.join(", ")) }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::Focus;
    pub fn foreground() -> Focus {
        Focus(0)
    }
    /// The OS hands focus back to the previous app when the overlay hides.
    pub fn restore(_: Focus) -> bool {
        true
    }
    pub fn element_at(_: i32, _: i32) -> Option<String> {
        None
    }
}

pub use imp::*;

pub fn paste() {
    use enigo::{Direction, Enigo, Key, Keyboard, Settings};
    let Ok(mut e) = Enigo::new(&Settings::default()) else { return };
    #[cfg(target_os = "macos")]
    let modifier = Key::Meta;
    #[cfg(not(target_os = "macos"))]
    let modifier = Key::Control;
    let _ = e.key(modifier, Direction::Press);
    let _ = e.key(Key::Unicode('v'), Direction::Click);
    let _ = e.key(modifier, Direction::Release);
}
