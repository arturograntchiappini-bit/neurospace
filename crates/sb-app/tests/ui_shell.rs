#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::fs;
use std::path::{Path, PathBuf};

use eframe::egui::{Key, Modifiers, Theme};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use sb_app::app::{AppOptions, SecondBrainApp};
use sb_app::state::settings::Settings;
use sb_app::ui::theme::Material;
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
