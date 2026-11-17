//! The operating-system calls behind the glass material (spec 9.6).

use eframe::egui::Theme;
use raw_window_handle::HasWindowHandle;

use crate::app::SecondBrainApp;

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

/// Brings the window material in step with the theme and the transparency setting.
///
/// Does nothing unless one of them changed since the last call.
pub fn sync_material(window: &impl HasWindowHandle, theme: Theme, app: &mut SecondBrainApp) {
    let dark = theme == Theme::Dark;
    if !app.material_update_due(dark) {
        return;
    }
    if app.reduce_transparency() {
        clear_mica(window);
    } else {
        app.set_material_available(apply_mica(window, dark));
    }
}
