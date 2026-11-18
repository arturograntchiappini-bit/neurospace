//! The operating-system calls behind the glass material (spec 9.6).

use raw_window_handle::HasWindowHandle;

/// Gives `window` the Windows Mica backdrop; false where the system or window refuses.
pub fn apply_mica(window: &impl HasWindowHandle, dark: bool) -> bool {
    #[cfg(windows)]
    {
        window_vibrancy::apply_mica(window, Some(dark)).is_ok()
    }
    #[cfg(not(windows))]
    {
        let _ = (window, dark);
        false
    }
}

/// Removes the Mica backdrop from `window`; nothing to do where there is none.
pub fn clear_mica(window: &impl HasWindowHandle) {
    #[cfg(windows)]
    {
        // A refusal means no Mica was applied, so there is nothing to clear.
        let _ = window_vibrancy::clear_mica(window);
    }
    #[cfg(not(windows))]
    let _ = window;
}
