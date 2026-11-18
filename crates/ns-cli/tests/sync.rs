#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use clap::Parser;
use ns_cli::cli::Cli;
use ns_sync::{StateDir, Ticket, to_hex};
use serde_json::{Value, json};
use tempfile::TempDir;

const POLL: Duration = Duration::from_millis(50);
const STARTUP: Duration = Duration::from_secs(60);
const SYNC: Duration = Duration::from_secs(20);
const LAN_CONFIG: &str = r#"{"mode":"lan","mdns":false}"#;

/// A vault and the state directory that belongs to it.
struct Device {
    _dirs: (TempDir, TempDir),
    vault: PathBuf,
    base: PathBuf,
    state: PathBuf,
}

fn device(config: Option<&str>) -> Device {
    let (vault_dir, base_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let (vault, base) = (
        vault_dir.path().to_path_buf(),
        base_dir.path().to_path_buf(),
    );
    let state = StateDir::for_vault_in(&base, &vault)
        .unwrap()
        .path()
        .to_path_buf();
    if let Some(config) = config {
        fs::write(state.join("config.json"), config).unwrap();
    }
    Device {
        _dirs: (vault_dir, base_dir),
        vault,
        base,
        state,
    }
}

/// A device whose state directory has not been created, to see which commands create it.
fn untouched_device() -> Device {
    let (vault_dir, base_dir) = (TempDir::new().unwrap(), TempDir::new().unwrap());
    let (vault, base) = (
        vault_dir.path().to_path_buf(),
        base_dir.path().to_path_buf(),
    );
    Device {
        _dirs: (vault_dir, base_dir),
        vault,
        state: base.join("vaults"),
        base,
    }
}

fn command(device: &Device, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_ns"));
    command
        .env_remove("NS_VAULT")
        .env("NS_STATE_DIR", &device.base)
        .arg("--vault")
        .arg(&device.vault)
        .args(args);
    command
}

fn ns_with_state(device: &Device, args: &[&str]) -> (i32, String, String) {
    let output = command(device, args).stdin(Stdio::null()).output().unwrap();
    (
        output.status.code().unwrap(),
        String::from_utf8(output.stdout).unwrap(),
        String::from_utf8(output.stderr).unwrap(),
    )
}

fn ok(device: &Device, args: &[&str]) -> String {
    let (code, stdout, stderr) = ns_with_state(device, args);
    assert_eq!(code, 0, "{args:?}: {stderr}");
    stdout
}

fn status_json(device: &Device) -> Value {
    serde_json::from_str(&ok(device, &["--json", "sync", "status"])).unwrap()
}

/// A running `ns sync run`. Dropping it kills the process, so a failing assertion never leaves one
/// holding the lock and the port.
struct Engine {
    child: Child,
    stdout: PathBuf,
    stderr: PathBuf,
}

impl Engine {
    fn spawn(device: &Device) -> Engine {
        let stdout = device.base.join("run.out");
        let stderr = device.base.join("run.err");
        let child = command(device, &["sync", "run"])
            .stdin(Stdio::null())
            .stdout(File::create(&stdout).unwrap())
            .stderr(File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        Engine {
            child,
            stdout,
            stderr,
        }
    }

    fn wait_exit(&mut self, limit: Duration) -> ExitStatus {
        wait_until(limit, "the engine to exit", || {
            self.child.try_wait().unwrap()
        })
    }

    /// Kills the process and returns what it printed on stdout and stderr.
    fn stop(mut self) -> (String, String) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let read = |path: &Path| fs::read_to_string(path).unwrap();
        (read(&self.stdout), read(&self.stderr))
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_until<T>(limit: Duration, what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(value) = probe() {
            return value;
        }
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

/// Waits until the engine of `device` publishes a status with an address; returns that status.
fn wait_for_status(device: &Device) -> Value {
    wait_until(STARTUP, "the engine's status", || {
        let (code, stdout, _) = ns_with_state(device, &["--json", "sync", "status"]);
        let status: Value = serde_json::from_str(&stdout).ok()?;
        (code == 0 && !status["addresses"].as_array()?.is_empty()).then_some(status)
    })
}

fn read_note(device: &Device, path: &str) -> Option<String> {
    let (code, stdout, _) = ns_with_state(device, &["read", path]);
    (code == 0).then_some(stdout)
}

#[test]
fn should_invite_before_any_engine_ran() {
    let a = device(Some(LAN_CONFIG));
    let (code, stdout, stderr) = ns_with_state(&a, &["sync", "invite"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stdout.starts_with("ns1/"), "{stdout}");
    let ticket: Ticket = stdout.trim().parse().unwrap();
    assert!(ticket.addrs.is_empty());
    assert!(stderr.contains("ns sync run"), "{stderr}");
    for file in ["device.key", "device.id", "secret"] {
        assert!(a.state.join(file).exists(), "{file}");
    }
    let _engine = Engine::spawn(&a);
    let status = wait_for_status(&a);
    assert_eq!(status["device_id"], json!(to_hex(&ticket.device_id)));
}

#[test]
fn should_report_engine_not_running() {
    let a = device(None);
    assert_eq!(
        ns_with_state(&a, &["sync", "status"]),
        (0, "engine not running\n".into(), String::new())
    );
    assert_eq!(
        ok(&a, &["--json", "sync", "status"]),
        "{\"running\":false}\n"
    );
}

#[test]
fn should_reject_a_malformed_ticket() {
    let a = untouched_device();
    let (code, stdout, stderr) = ns_with_state(&a, &["sync", "join", "nonsense"]);
    assert_eq!((code, stdout.as_str()), (1, ""));
    assert!(stderr.contains("invalid ticket"), "{stderr}");
    assert_eq!(
        fs::read_dir(&a.base).unwrap().count(),
        0,
        "state was created"
    );
}

#[test]
fn should_not_print_the_secret_of_a_ticket_argument_in_debug_output() {
    let ticket = Ticket {
        device_id: [9; 32],
        secret: [8; 32],
        addrs: Vec::new(),
    };
    let args = ["ns", "sync", "join", &ticket.to_string()];
    let cli = Cli::try_parse_from(args).unwrap();
    let shown = format!("{cli:?}");
    assert!(shown.contains("Join"), "{shown}");
    assert!(!shown.contains(&to_hex(&ticket.secret)), "{shown}");
}

#[test]
fn should_print_the_ticket_as_json_when_inviting() {
    let a = device(Some(LAN_CONFIG));
    let document: Value = serde_json::from_str(&ok(&a, &["--json", "sync", "invite"])).unwrap();
    let ticket: Ticket = document["ticket"].as_str().unwrap().parse().unwrap();
    assert!(ticket.addrs.is_empty());
}

#[test]
fn should_queue_a_join_for_the_running_engine() {
    let a = device(Some(LAN_CONFIG));
    let _engine = Engine::spawn(&a);
    wait_for_status(&a);
    let ticket = Ticket {
        device_id: [9; 32],
        secret: [8; 32],
        addrs: Vec::new(),
    };
    let joined = ok(&a, &["--json", "sync", "join", &ticket.to_string()]);
    assert_eq!(joined, "{\"applied\":\"queued\"}\n");
    // The engine removes the file before it saves the secret, so wait for the effect, not the file.
    wait_until(SYNC, "the engine to carry out the join", || {
        (a_secret(&a) == [8; 32]).then_some(())
    });
}

#[test]
fn should_apply_join_and_leave_without_an_engine() {
    let a = device(None);
    let ticket = Ticket {
        device_id: [9; 32],
        secret: [8; 32],
        addrs: Vec::new(),
    };
    let joined = ok(&a, &["sync", "join", &ticket.to_string()]);
    assert!(joined.contains("applied now"), "{joined}");
    assert!(a.state.join("secret").exists());
    assert_eq!(
        ok(&a, &["--json", "sync", "leave"]),
        "{\"applied\":\"now\"}\n"
    );
    assert!(!a.state.join("secret").exists());
}

#[test]
fn should_refuse_a_second_run_for_the_same_vault() {
    let a = device(Some(LAN_CONFIG));
    let _engine = Engine::spawn(&a);
    wait_for_status(&a);
    let (code, _, stderr) = ns_with_state(&a, &["sync", "run"]);
    assert_eq!(code, 1);
    assert!(stderr.contains("already running"), "{stderr}");
}

#[test]
fn should_stop_the_engine_when_a_leave_is_queued() {
    let a = device(Some(LAN_CONFIG));
    let mut engine = Engine::spawn(&a);
    wait_for_status(&a);
    assert_eq!(
        ok(&a, &["--json", "sync", "leave"]),
        "{\"applied\":\"queued\"}\n"
    );
    assert!(engine.wait_exit(SYNC).success());
    assert!(!a.state.join("secret").exists());
}

#[cfg(unix)]
#[test]
fn should_stop_cleanly_on_sigterm() {
    let a = device(Some(LAN_CONFIG));
    let mut engine = Engine::spawn(&a);
    wait_for_status(&a);
    let pid = engine.child.id().to_string();
    let signalled = Command::new("kill").args(["-TERM", &pid]).status().unwrap();
    assert!(signalled.success());
    assert!(engine.wait_exit(SYNC).success());
}

fn pair(a: &Device, b: &Device) -> (Engine, Engine) {
    let engine_a = Engine::spawn(a);
    let status = wait_for_status(a);
    let port = status["addresses"][0]
        .as_str()
        .unwrap()
        .rsplit(':')
        .next()
        .unwrap()
        .to_owned();
    let mut ticket: Ticket = ok(a, &["sync", "invite"]).trim().parse().unwrap();
    ticket.addrs = vec![format!("127.0.0.1:{port}").parse().unwrap()];
    ok(b, &["sync", "join", &ticket.to_string()]);
    (engine_a, Engine::spawn(b))
}

#[test]
fn should_sync_two_processes_paired_by_ticket() {
    let (a, b) = (device(Some(LAN_CONFIG)), device(Some(LAN_CONFIG)));
    let (engine_a, engine_b) = pair(&a, &b);
    let device_b = wait_for_status(&b)["device_id"].clone();

    ok(&a, &["write", "hello.md", "--content", "from A"]);
    wait_until(SYNC, "A's note in B's vault", || {
        read_note(&b, "hello.md").filter(|text| text == "from A")
    });
    ok(&b, &["append", "hello.md", "--content", "from B"]);
    wait_until(SYNC, "B's append in A's vault", || {
        read_note(&a, "hello.md").filter(|text| text.ends_with("from B"))
    });

    wait_until(SYNC, "B connected in A's status", || {
        let status = status_json(&a);
        let connected = |peer: &Value| peer["id"] == device_b && peer["connected"] == json!(true);
        status["peers"]
            .as_array()?
            .iter()
            .any(connected)
            .then_some(())
    });

    let secret = to_hex(&a_secret(&a));
    for (stdout, stderr) in [engine_a.stop(), engine_b.stop()] {
        assert_eq!(stdout, "");
        assert!(stderr.starts_with("device "), "{stderr}");
        assert!(!stderr.contains(&secret), "the secret reached the log");
    }
}

fn a_secret(device: &Device) -> [u8; 32] {
    StateDir::at(&device.state)
        .unwrap()
        .secret()
        .unwrap()
        .unwrap()
}
