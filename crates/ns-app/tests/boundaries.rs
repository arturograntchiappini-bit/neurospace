#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::fs;
use std::path::Path;

const FORBIDDEN: [&str; 4] = ["egui", "eframe", "rfd", "window_vibrancy"];

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn should_keep_state_module_free_of_ui_crates() {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src/state"),
        &mut files,
    );
    assert!(!files.is_empty(), "no source files found under src/state");
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        for word in FORBIDDEN {
            assert!(
                !text.contains(word),
                "{} mentions `{word}`: src/state must stay free of UI crates",
                file.display()
            );
        }
    }
}
