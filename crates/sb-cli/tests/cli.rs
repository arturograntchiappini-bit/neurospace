#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};
use tempfile::TempDir;

fn command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_sb"));
    command.env_remove("SB_VAULT");
    command
}

fn finish(mut command: Command, stdin: Option<&str>) -> (i32, String, String) {
    command
        .stdin(if stdin.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command.spawn().unwrap();
    if let Some(text) = stdin {
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
    }
    let output = child.wait_with_output().unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn sb(vault: &Path, args: &[&str], stdin: Option<&str>) -> (i32, String, String) {
    let mut cmd = command();
    cmd.arg("--vault").arg(vault).args(args);
    finish(cmd, stdin)
}

fn sb_json(vault: &Path, args: &[&str]) -> Value {
    let (code, stdout, stderr) = sb(vault, args, None);
    assert_eq!(code, 0, "{args:?}: {stderr}");
    assert!(stdout.ends_with('\n'), "{args:?}: {stdout:?}");
    serde_json::from_str(&stdout).unwrap()
}

fn seeded() -> TempDir {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("a.md"), "hello [[b]]").unwrap();
    fs::write(tmp.path().join("b.md"), "piped\r\n").unwrap();
    tmp
}

#[test]
fn should_write_from_flag_and_from_stdin() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    let flag = sb(vault, &["write", "a.md", "--content", "hello [[b]]"], None);
    assert_eq!(flag.0, 0);
    assert_eq!(sb(vault, &["write", "b.md"], Some("piped\r\n")).0, 0);
    assert_eq!(sb(vault, &["read", "a.md"], None).1, "hello [[b]]");
    assert_eq!(sb(vault, &["read", "b.md"], None).1, "piped\r\n");
}

#[test]
fn should_list_notes_as_lines_and_as_json() {
    let tmp = seeded();
    let vault = tmp.path();
    assert_eq!(sb(vault, &["list"], None).1, "a.md\nb.md\n");
    let expected = json!({
        "notes": [{"path": "a.md", "title": "a"}, {"path": "b.md", "title": "b"}],
        "ignored": 0
    });
    assert_eq!(sb_json(vault, &["--json", "list"]), expected);
    assert_eq!(sb_json(vault, &["list", "--json"]), expected);
}

#[test]
fn should_print_json_for_every_query_command() {
    let tmp = seeded();
    let vault = tmp.path();
    let links = sb_json(vault, &["--json", "links", "b.md"]);
    assert_eq!(links["backlinks"][0]["from"], "a.md");
    let graph = sb_json(vault, &["--json", "graph"]);
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(graph["edges"], json!([{"from": "a.md", "to": "b.md"}]));
    let around = sb_json(
        vault,
        &["--json", "graph", "--around", "a.md", "--depth", "1"],
    );
    assert_eq!(around["nodes"].as_array().unwrap().len(), 2);
    let search = sb_json(vault, &["--json", "search", "hello"]);
    assert_eq!(search["hits"][0]["path"], "a.md");
    assert_eq!(sb_json(vault, &["--json", "tags"]), json!({"tags": []}));
}

#[test]
fn should_print_json_for_every_mutating_command() {
    let tmp = seeded();
    let vault = tmp.path();
    let steps = [
        (
            ["write", "c.md", "--content", "x"].as_slice(),
            json!({"path": "c.md", "created": true}),
        ),
        (
            ["append", "c.md", "--content", "y"].as_slice(),
            json!({"path": "c.md", "created": false}),
        ),
        (
            ["rename", "c.md", "d/e.md"].as_slice(),
            json!({"from": "c.md", "to": "d/e.md"}),
        ),
        (
            ["delete", "d/e.md"].as_slice(),
            json!({"path": "d/e.md", "trashed": true}),
        ),
    ];
    for (args, expected) in steps {
        let full = [["--json"].as_slice(), args].concat();
        assert_eq!(sb_json(vault, &full), expected, "{args:?}");
    }
}

#[test]
fn should_pin_the_json_shape_of_links_and_search() {
    let tmp = TempDir::new().unwrap();
    let vault = tmp.path();
    fs::write(vault.join("a.md"), "hello [[b#H|al]]").unwrap();
    fs::write(vault.join("b.md"), "x").unwrap();
    let (_, links, _) = sb(vault, &["--json", "links", "a.md"], None);
    assert_eq!(
        links,
        concat!(
            r#"{"outgoing":[{"target":"b","heading":"H","alias":"al","kind":"wiki","line":1,"resolved":"b.md"}],"#,
            r#""backlinks":[]}"#,
            "\n"
        )
    );
    let (_, links, _) = sb(vault, &["--json", "links", "b.md"], None);
    assert_eq!(
        links,
        concat!(
            r#"{"outgoing":[],"backlinks":[{"from":"a.md","line":1,"context":"hello [[b#H|al]]"}]}"#,
            "\n"
        )
    );
    let (_, search, _) = sb(vault, &["--json", "search", "hello"], None);
    assert_eq!(
        search,
        concat!(
            r#"{"hits":[{"path":"a.md","title":"a","score":1,"#,
            r#""snippets":[{"line":1,"text":"hello [[b#H|al]]"}]}]}"#,
            "\n"
        )
    );
}

#[test]
fn should_print_readable_text_without_json() {
    let tmp = seeded();
    let vault = tmp.path();
    let search = sb(vault, &["search", "hello"], None).1;
    assert!(
        search.contains("a.md") && search.contains("hello [[b]]"),
        "{search}"
    );
    assert!(sb(vault, &["links", "b.md"], None).1.contains("a.md"));
    assert!(sb(vault, &["graph"], None).1.contains("a.md -> b.md"));
}

#[test]
fn should_exit_1_with_message_on_operation_error() {
    let tmp = seeded();
    let vault = tmp.path();
    let (code, stdout, stderr) = sb(vault, &["read", "../x.md"], None);
    assert_eq!((code, stdout.as_str()), (1, ""));
    assert!(stderr.contains("invalid note path"), "{stderr}");
    let (code, _, stderr) = sb(vault, &["read", "missing.md"], None);
    assert_eq!(code, 1);
    assert!(stderr.contains("note not found: missing.md"), "{stderr}");
    let (code, _, stderr) = sb(&vault.join("nope"), &["list"], None);
    assert_eq!(code, 1);
    assert!(stderr.contains("is not a directory"), "{stderr}");
}

#[test]
fn should_exit_2_on_usage_error() {
    let mut cmd = command();
    cmd.arg("frobnicate");
    assert_eq!(finish(cmd, None).0, 2);
}

#[test]
fn should_use_sb_vault_environment_variable() {
    let tmp = seeded();
    let mut cmd = command();
    cmd.env("SB_VAULT", tmp.path()).arg("list");
    assert_eq!(finish(cmd, None).1, "a.md\nb.md\n");
}

#[test]
fn should_summarise_ignored_files_on_stderr() {
    let tmp = seeded();
    fs::write(tmp.path().join("bin.md"), [0xff]).unwrap();
    let (code, stdout, stderr) = sb(tmp.path(), &["list"], None);
    assert_eq!((code, stdout.as_str()), (0, "a.md\nb.md\n"));
    assert!(stderr.contains("1 file ignored"), "{stderr}");
    assert!(stderr.contains("bin.md"), "{stderr}");
}

#[test]
fn should_handle_empty_stdin() {
    let tmp = TempDir::new().unwrap();
    assert_eq!(sb(tmp.path(), &["write", "empty.md"], Some("")).0, 0);
    assert_eq!(fs::metadata(tmp.path().join("empty.md")).unwrap().len(), 0);
}
