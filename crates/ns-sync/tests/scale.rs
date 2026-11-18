#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use ns_core::{NotePath, Vault};
use ns_sync::{LocalReplica, StateDir};

const NOTES: usize = 10_000;
const PER_FOLDER: usize = 100;

fn note_rel(i: usize) -> String {
    format!("f{:02}/n{i:04}.md", i / PER_FOLDER)
}

/// About 10 KB: 100 lines of 100 characters; line 1 is the title, line 2 links to the next note.
fn note_text(i: usize) -> String {
    let mut lines = vec![
        format!("# Note {i}"),
        format!("see [[n{:04}]]", (i + 1) % NOTES),
    ];
    lines.extend((2..100).map(|l| format!("line {l} of note {i} ")));
    lines.iter().map(|s| format!("{s:.<99}\n")).collect()
}

fn write_vault(root: &Path) {
    fs::create_dir_all(root.join(".neurospace")).unwrap();
    for i in 0..NOTES {
        let path = root.join(note_rel(i));
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, note_text(i)).unwrap();
    }
}

fn timed(replica: &mut LocalReplica) -> Duration {
    let start = Instant::now();
    replica.reconcile().unwrap();
    start.elapsed()
}

fn ms(d: Duration) -> u128 {
    d.as_millis()
}

#[test]
#[ignore = "scale timing; run in release"]
fn should_reconcile_ten_thousand_notes_within_budget() {
    let vault_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    write_vault(vault_dir.path());
    let vault = Vault::open(vault_dir.path()).unwrap();
    let mut replica = LocalReplica::open(vault, StateDir::at(state_dir.path()).unwrap()).unwrap();

    let first = timed(&mut replica);
    let mut quiet = [
        timed(&mut replica),
        timed(&mut replica),
        timed(&mut replica),
    ];

    let edited = note_rel(4242);
    assert_eq!(edited, "f42/n4242.md");
    let full = vault_dir.path().join(&edited);
    fs::write(&full, format!("{}edited\n", note_text(4242))).unwrap();
    let one_change = timed(&mut replica);

    eprintln!(
        "scale: first={} quiet={},{},{} one_change={}",
        ms(first),
        ms(quiet[0]),
        ms(quiet[1]),
        ms(quiet[2]),
        ms(one_change)
    );

    let view = replica.doc.view();
    let note = view.get(&NotePath::parse(&edited).unwrap().key()).unwrap();
    assert!(note.text.ends_with("edited\n"));
    quiet.sort();
    assert!(quiet[1] < Duration::from_millis(300), "quiet median");
    assert!(one_change < Duration::from_millis(500), "one change");
}
