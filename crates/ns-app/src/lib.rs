//! Neurospace desktop application: state in `state`, drawing in `ui`.

pub mod app;
pub mod platform;
pub mod state;
pub mod sync_control;
mod sync_host;
pub mod ui;

/// Application id the window sets; the Linux `.desktop` file's `StartupWMClass` must equal it.
pub const APP_ID: &str = "neurospace";

#[cfg(test)]
mod tests {
    use super::APP_ID;

    #[test]
    fn should_match_the_desktop_file_to_the_window_app_id() {
        let desktop = include_str!("../../../packaging/linux/neurospace.desktop");
        let expected = [
            "[Desktop Entry]",
            "Type=Application",
            "Name=Neurospace",
            "Exec=neurospace %f",
            "Icon=neurospace",
            "Terminal=false",
            "Categories=Office;",
            &format!("StartupWMClass={APP_ID}"),
        ];
        for line in expected {
            assert!(desktop.lines().any(|l| l == line), "missing line: {line}");
        }
    }
}
