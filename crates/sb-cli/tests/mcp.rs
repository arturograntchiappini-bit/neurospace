#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use rmcp::ServiceExt;
use rmcp::model::CallToolRequestParams;
use rmcp::service::{RoleClient, RunningService};
use sb_cli::mcp::NotesServer;
use sb_core::{CoreError, Vault, ops};
use serde::Serialize;
use serde_json::{Value, json};
use tempfile::TempDir;

type Client = RunningService<RoleClient, ()>;

fn seeded() -> TempDir {
    let tmp = TempDir::new().unwrap();
    fs::write(tmp.path().join("a.md"), "hello [[b]] #t").unwrap();
    fs::write(tmp.path().join("B.md"), "world").unwrap();
    tmp
}

async fn connect(vault_dir: &Path) -> Client {
    let (server_io, client_io) = tokio::io::duplex(64 * 1024);
    let server = NotesServer::new(Vault::open(vault_dir).unwrap());
    tokio::spawn(async move { server.serve(server_io).await.unwrap().waiting().await });
    ().serve(client_io).await.unwrap()
}

async fn call(client: &Client, tool: &str, args: Value) -> (bool, String) {
    let Value::Object(arguments) = args else {
        panic!("tool arguments must be a JSON object");
    };
    let request = CallToolRequestParams::new(tool.to_owned()).with_arguments(arguments);
    let result = client.call_tool(request).await.unwrap();
    let text = result.content[0].as_text().unwrap().text.clone();
    (result.is_error.unwrap(), text)
}

async fn call_json(client: &Client, tool: &str, args: Value) -> Value {
    let (is_error, text) = call(client, tool, args).await;
    assert!(!is_error, "{tool}: {text}");
    serde_json::from_str(&text).unwrap()
}

#[tokio::test]
async fn should_list_exactly_the_ten_tools() {
    let tmp = seeded();
    let client = connect(tmp.path()).await;
    let tools = client.list_all_tools().await.unwrap();
    let mut names: Vec<&str> = tools.iter().map(|tool| tool.name.as_ref()).collect();
    names.sort_unstable();
    assert_eq!(
        names,
        [
            "append_to_note",
            "delete_note",
            "get_graph",
            "get_links",
            "list_notes",
            "list_tags",
            "read_note",
            "rename_note",
            "search_notes",
            "write_note"
        ]
    );
    for tool in &tools {
        let description = tool.description.as_deref().unwrap_or_default();
        assert!(!description.is_empty(), "{} has no description", tool.name);
    }
    let read_note = tools.iter().find(|tool| tool.name == "read_note").unwrap();
    let schema = Value::Object((*read_note.input_schema).clone());
    let description = schema["properties"]["path"]["description"].as_str();
    assert!(!description.unwrap_or_default().is_empty(), "{schema}");
    assert_eq!(schema["required"], json!(["path"]));
}

#[tokio::test]
async fn should_report_server_info_and_instructions() {
    let tmp = seeded();
    let client = connect(tmp.path()).await;
    let info = client.peer_info().unwrap();
    assert_eq!(info.server_info.as_ref().unwrap().name, "second-brain");
    let instructions = info.instructions.as_deref().unwrap();
    for needle in ["Markdown", "[[", ".md"] {
        assert!(instructions.contains(needle), "{needle}: {instructions}");
    }
}

#[tokio::test]
async fn should_read_note_as_raw_text() {
    let tmp = seeded();
    let client = connect(tmp.path()).await;
    let outcome = call(&client, "read_note", json!({ "path": "a.md" })).await;
    assert_eq!(outcome, (false, "hello [[b]] #t".to_owned()));
}

fn value<T: Serialize>(result: Result<T, CoreError>) -> Value {
    serde_json::to_value(result.unwrap()).unwrap()
}

/// Every read-only tool call paired with what the matching core operation returns.
fn core_cases(vault: &Vault) -> Vec<(&'static str, Value, Value)> {
    vec![
        ("list_notes", json!({}), value(ops::list_notes(vault, None))),
        (
            "list_notes",
            json!({ "folder": "x" }),
            value(ops::list_notes(vault, Some("x"))),
        ),
        (
            "search_notes",
            json!({ "query": "hello" }),
            value(ops::search_notes(vault, "hello", None)),
        ),
        (
            "get_links",
            json!({ "path": "b.md" }),
            value(ops::get_links(vault, "b.md")),
        ),
        (
            "get_graph",
            json!({}),
            value(ops::get_graph(vault, None, None)),
        ),
        (
            "get_graph",
            json!({ "around": "a.md", "depth": 1 }),
            value(ops::get_graph(vault, Some("a.md"), Some(1))),
        ),
        ("list_tags", json!({}), value(ops::list_tags(vault))),
    ]
}

#[tokio::test]
async fn should_return_json_matching_core_operations() {
    let tmp = seeded();
    let client = connect(tmp.path()).await;
    let vault = Vault::open(tmp.path()).unwrap();
    for (tool, args, expected) in core_cases(&vault) {
        assert_eq!(call_json(&client, tool, args).await, expected, "{tool}");
    }
}

#[tokio::test]
async fn should_write_append_rename_and_delete() {
    let tmp = seeded();
    let client = connect(tmp.path()).await;
    let note = tmp.path().join("n/new.md");

    let written = call_json(
        &client,
        "write_note",
        json!({ "path": "n/new.md", "content": "x" }),
    )
    .await;
    assert_eq!(written, json!({ "path": "n/new.md", "created": true }));
    assert_eq!(fs::read_to_string(&note).unwrap(), "x");

    let appended = call_json(
        &client,
        "append_to_note",
        json!({ "path": "n/new.md", "content": "y" }),
    )
    .await;
    assert_eq!(appended, json!({ "path": "n/new.md", "created": false }));
    assert_eq!(fs::read_to_string(&note).unwrap(), "x\ny");

    let renamed = call_json(
        &client,
        "rename_note",
        json!({ "from": "n/new.md", "to": "m.md" }),
    )
    .await;
    assert_eq!(renamed, json!({ "from": "n/new.md", "to": "m.md" }));

    let deleted = call_json(&client, "delete_note", json!({ "path": "m.md" })).await;
    assert_eq!(deleted, json!({ "path": "m.md", "trashed": true }));
    let trashed = fs::read_dir(tmp.path().join(".secondbrain/trash")).unwrap();
    assert_eq!(trashed.count(), 1);
}

#[tokio::test]
async fn should_return_tool_error_for_bad_paths_and_missing_notes() {
    let tmp = seeded();
    let client = connect(tmp.path()).await;

    let (is_error, text) = call(&client, "read_note", json!({ "path": "../secret.md" })).await;
    assert!(is_error && text.contains("invalid note path"), "{text}");

    let (is_error, text) = call(&client, "read_note", json!({ "path": "nope.md" })).await;
    assert!(is_error && text.contains("note not found"), "{text}");

    let forbidden = json!({ "path": ".secondbrain/x.md", "content": "x" });
    let (is_error, text) = call(&client, "write_note", forbidden).await;
    assert!(is_error, "{text}");
    assert!(!tmp.path().join(".secondbrain").exists());
}

#[tokio::test]
async fn should_return_tool_error_for_wrong_or_missing_arguments() {
    let tmp = seeded();
    let client = connect(tmp.path()).await;
    let bad_calls = [
        ("read_note", json!({})),
        ("read_note", json!({ "path": 5 })),
        ("search_notes", json!({ "query": "x", "limit": "many" })),
        ("get_graph", json!({ "depth": 300 })),
    ];
    for (tool, args) in bad_calls {
        let (is_error, text) = call(&client, tool, args.clone()).await;
        assert!(is_error, "{tool} {args}: {text}");
    }
    let outcome = call(&client, "read_note", json!({ "path": "a.md" })).await;
    assert_eq!(outcome, (false, "hello [[b]] #t".to_owned()));
}

#[tokio::test]
async fn should_carry_large_notes() {
    let tmp = seeded();
    let client = connect(tmp.path()).await;
    let big = "a".repeat(2 * 1024 * 1024);
    let (is_error, _) = call(
        &client,
        "write_note",
        json!({ "path": "big.md", "content": big }),
    )
    .await;
    assert!(!is_error);
    let outcome = call(&client, "read_note", json!({ "path": "big.md" })).await;
    assert_eq!(outcome, (false, big));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn should_keep_every_text_when_appends_run_concurrently() {
    const APPENDS: usize = 40;
    let tmp = seeded();
    let client = connect(tmp.path()).await;
    let mut calls = tokio::task::JoinSet::new();
    for n in 0..APPENDS {
        let peer = client.peer().clone();
        let arguments = json!({ "path": "log.md", "content": format!("line-{n}") });
        let Value::Object(arguments) = arguments else {
            unreachable!()
        };
        calls.spawn(async move {
            let request = CallToolRequestParams::new("append_to_note").with_arguments(arguments);
            peer.call_tool(request).await.unwrap().is_error
        });
    }
    while let Some(outcome) = calls.join_next().await {
        assert_eq!(outcome.unwrap(), Some(false));
    }
    let (_, text) = call(&client, "read_note", json!({ "path": "log.md" })).await;
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), APPENDS, "{text}");
    for n in 0..APPENDS {
        let wanted = format!("line-{n}");
        assert_eq!(lines.iter().filter(|line| **line == wanted).count(), 1);
    }
}

const SESSION: [&str; 4] = [
    r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"t","version":"0"}}}"#,
    r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
    r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
    r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"read_note","arguments":{"path":"a.md"}}}"#,
];

/// Runs `sb mcp`, feeds it `SESSION`, closes stdin and returns every stdout line as JSON.
fn run_session(vault_dir: &Path) -> Vec<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_sb"))
        .env_remove("SB_VAULT")
        .arg("--vault")
        .arg(vault_dir)
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    for line in SESSION {
        writeln!(stdin, "{line}").unwrap();
    }
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8(output.stdout).unwrap();
    stdout
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn should_speak_only_protocol_on_stdout() {
    let tmp = seeded();
    let replies = run_session(tmp.path());
    assert!(replies.iter().all(|reply| reply["jsonrpc"] == "2.0"));
    let result = |id: u64| {
        let reply = replies.iter().find(|reply| reply["id"] == id).unwrap();
        reply["result"].clone()
    };
    for id in [1, 2, 3] {
        assert!(result(id).is_object(), "id {id}");
    }
    assert_eq!(result(1)["protocolVersion"], "2025-06-18");
    assert_eq!(result(1)["serverInfo"]["name"], "second-brain");
    assert_eq!(result(3)["content"][0]["text"], "hello [[b]] #t");
}
