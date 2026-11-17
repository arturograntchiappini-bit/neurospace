#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use eframe::App;
use eframe::egui::{Key, Modifiers, Role, Theme, Visuals, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sb_app::app::{AppOptions, SecondBrainApp};
use sb_app::state::settings::Settings;
use sb_app::ui::theme::{self, Material};
use tempfile::TempDir;

use common::{harness, harness_from, run_until};

fn make_vault(parent: &Path) -> PathBuf {
    let vault = parent.join("my-vault");
    fs::create_dir_all(&vault).unwrap();
    fs::write(vault.join("Alpha.md"), "# Alpha\n").unwrap();
    fs::write(vault.join("Beta.md"), "see [[Alpha]]\n").unwrap();
    vault
}

/// A harness already showing a vault; the directories must outlive it.
fn open_vault_harness() -> (Harness<'static, SecondBrainApp>, TempDir, TempDir) {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    let (mut harness, dir) = harness(Some(&vault));
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    (harness, dir, scratch)
}

fn type_path(harness: &mut Harness<'static, SecondBrainApp>, text: &str) {
    harness.get_by_label("Vault folder path").focus();
    harness.run();
    harness.get_by_label("Vault folder path").type_text(text);
    harness.run();
}

fn saved_json(dir: &TempDir) -> serde_json::Value {
    serde_json::from_str(&fs::read_to_string(dir.path().join("app.json")).unwrap()).unwrap()
}

#[test]
fn should_show_welcome_when_no_vault_known() {
    let (mut harness, _dir) = harness(None);
    harness.run();
    assert_eq!(harness.state().screen_name(), "welcome");
    harness.get_by_label("Open a folder");
    harness.get_by_label("Open");
}

#[test]
fn should_open_vault_from_path_field() {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    let (mut harness, dir) = harness(None);
    harness.run();
    type_path(&mut harness, vault.to_str().unwrap());
    harness.get_by_label("Open").click();
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    harness.get_by_label("my-vault");
    let saved = Settings::load(&dir.path().join("app.json"));
    assert_eq!(saved.last_vault, Some(vault));
}

#[test]
fn should_show_error_and_stay_on_welcome_when_path_is_not_a_folder() {
    let scratch = tempfile::tempdir().unwrap();
    let missing = scratch.path().join("nothing-here");
    let (mut harness, _dir) = harness(None);
    harness.run();
    type_path(&mut harness, missing.to_str().unwrap());
    harness.get_by_label("Open").click();
    run_until(&mut harness, "failed open", |app| {
        app.screen_name() == "welcome" && app.welcome_error().is_some()
    });
    harness.run();
    let error = harness.state().welcome_error().unwrap().to_owned();
    assert!(error.starts_with("Could not open the vault"), "{error}");
    harness.get_by_label_contains("Could not open the vault");
}

#[test]
fn should_reopen_last_vault_on_start() {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("app.json");
    let settings = Settings {
        last_vault: Some(vault),
        ..Settings::default()
    };
    settings.save(&settings_path).unwrap();
    let mut harness = harness_from(AppOptions {
        settings_path,
        state_base: dir.path().join("state"),
        vault: None,
    });
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    assert!(harness.state().workspace().is_some());
}

#[test]
fn should_switch_to_light_theme_from_menu() {
    let (mut harness, dir, _scratch) = open_vault_harness();
    harness.get_by_label("Menu").click();
    harness.run();
    harness.get_by_label("Light").click();
    harness.run();
    assert_eq!(harness.ctx.theme(), Theme::Light);
    assert_eq!(saved_json(&dir)["theme"], "light");
}

#[test]
fn should_use_opaque_material_when_reduce_transparency_is_on() {
    let (mut harness, dir, _scratch) = open_vault_harness();
    assert_eq!(harness.state().material(), Material::Simulated);
    harness.get_by_label("Menu").click();
    harness.run();
    harness.get_by_label("Reduce transparency").click();
    harness.run();
    assert_eq!(harness.state().material(), Material::Opaque);
    assert!(Settings::load(&dir.path().join("app.json")).reduce_transparency);
}

#[test]
fn should_change_zoom_with_keyboard_and_remember_it() {
    let (mut harness, dir, _scratch) = open_vault_harness();
    for _ in 0..2 {
        harness.key_press_modifiers(Modifiers::COMMAND, Key::Plus);
        harness.run();
    }
    assert!((harness.ctx.zoom_factor() - 1.2).abs() < 1e-4);
    assert!((Settings::load(&dir.path().join("app.json")).zoom - 1.2).abs() < 1e-4);
}

#[test]
fn should_dismiss_error_notice() {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    let blocker = scratch.path().join("not-a-folder");
    fs::write(&blocker, "x").unwrap();
    // Saving settings fails: the parent of `app.json` is a file.
    let mut harness = harness_from(AppOptions {
        settings_path: blocker.join("app.json"),
        state_base: scratch.path().join("state"),
        vault: Some(vault),
    });
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    harness.get_by_label_contains("Could not save settings");
    harness.get_by_label("Dismiss").click();
    harness.run();
    assert!(
        harness
            .query_by_label_contains("Could not save settings")
            .is_none()
    );
}

fn toggle_hides_and_shows(toggle: &str, section: &str) {
    let (mut harness, _dir, _scratch) = open_vault_harness();
    harness.get_by_label(section);
    harness.get_by_role_and_label(Role::Button, toggle).click();
    harness.run();
    assert!(
        harness.query_by_label(section).is_none(),
        "{section} still shown"
    );
    harness.get_by_role_and_label(Role::Button, toggle).click();
    harness.run();
    harness.get_by_label(section);
}

#[test]
fn should_hide_and_show_left_panel_with_sidebar_toggle() {
    toggle_hides_and_shows("Toggle sidebar", "FILES");
}

#[test]
fn should_hide_and_show_right_panel_with_inspector_toggle() {
    toggle_hides_and_shows("Toggle inspector", "OUTGOING LINKS");
}

#[test]
fn should_open_new_note_dialog_from_top_bar_button() {
    let (mut harness, _dir, _scratch) = open_vault_harness();
    assert!(harness.query_by_label("Note name").is_none());
    harness.get_by_label("New").click();
    harness.run();
    harness.get_by_label("Note name");
}

#[test]
fn should_choose_material_and_clear_colour_from_availability_and_setting() {
    let (mut harness, _dir, _scratch) = open_vault_harness();
    let dark = Visuals::dark();
    let backdrop = |theme, material| {
        let top = theme::palette(theme, material).backdrop_top;
        top.to_color32().to_normalized_gamma_f32()
    };
    let simulated = backdrop(Theme::Dark, Material::Simulated);
    assert_eq!(harness.state().clear_color(&dark), simulated);
    harness.state_mut().set_material_available(true);
    assert_eq!(harness.state().material(), Material::Mica);
    assert_eq!(harness.state().clear_color(&dark), [0.0; 4]);
    harness.get_by_label("Menu").click();
    harness.run();
    harness.get_by_label("Reduce transparency").click();
    harness.run();
    assert_eq!(harness.state().material(), Material::Opaque);
    let opaque = backdrop(Theme::Light, Material::Opaque);
    assert_eq!(harness.state().clear_color(&Visuals::light()), opaque);
}

/// A vault window at text size 200 % that is 360 x 240 points (720 x 480 px).
fn small_zoomed_harness() -> (Harness<'static, SecondBrainApp>, TempDir, TempDir) {
    let scratch = tempfile::tempdir().unwrap();
    let vault = make_vault(scratch.path());
    let dir = tempfile::tempdir().unwrap();
    let settings_path = dir.path().join("app.json");
    let settings = Settings {
        zoom: 2.0,
        ..Settings::default()
    };
    settings.save(&settings_path).unwrap();
    let options = AppOptions {
        settings_path,
        state_base: dir.path().join("state"),
        vault: Some(vault),
    };
    let mut harness = harness_from(options);
    harness.set_size(vec2(360.0, 240.0));
    (harness, dir, scratch)
}

#[test]
fn should_keep_side_panels_within_thirty_percent_of_a_small_zoomed_window() {
    let (mut harness, _dir, _scratch) = small_zoomed_harness();
    run_until(&mut harness, "vault screen", |app| {
        app.screen_name() == "vault"
    });
    harness.run();
    let width = harness.ctx.content_rect().width();
    // Node rectangles are in points, like `width`.
    let search = harness.get_by_label("Search notes").rect();
    let links = harness.get_by_label("OUTGOING LINKS").rect();
    let (left_edge, right_edge) = (search.right(), links.left());
    assert!(
        left_edge <= 0.3 * width,
        "left panel too wide: {left_edge} of {width}"
    );
    assert!(
        right_edge >= 0.7 * width,
        "right panel too wide: {right_edge} of {width}"
    );
}
