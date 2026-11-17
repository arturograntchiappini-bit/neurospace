#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

use std::fs;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::time::{Duration, Instant};

use iroh::endpoint::{ConnectionError, VarInt, presets};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use sb_sync::{ALPN, EngineHandle, PeerEntry, StateDir, Ticket, start_engine, to_hex};
use tempfile::TempDir;

const CONNECT_LIMIT: Duration = Duration::from_secs(30);
const FILE_LIMIT: Duration = Duration::from_secs(10);
const POLL: Duration = Duration::from_millis(50);
/// Mirrors `HANDSHAKE_TIMEOUT` (10 s) of the network adapter, which the crate keeps private.
const HANDSHAKE_SECS: u64 = 10;
/// The close code the adapter uses for a handshake it refuses.
const CLOSE_REFUSED: u32 = 1;

struct Rig {
    vault: TempDir,
    state: TempDir,
    engine: EngineHandle,
}

impl Rig {
    fn start() -> Rig {
        Self::start_in(tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap())
    }

    fn start_internet() -> Rig {
        let (vault, state) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
        let config = r#"{"mode":"internet","mdns":false}"#;
        fs::write(state.path().join("config.json"), config).unwrap();
        let engine = start_engine(vault.path(), StateDir::at(state.path()).unwrap(), None).unwrap();
        Rig {
            vault,
            state,
            engine,
        }
    }

    fn start_in(vault: TempDir, state: TempDir) -> Rig {
        let config = state.path().join("config.json");
        fs::write(config, r#"{"mode":"lan","mdns":false}"#).unwrap();
        let engine = start_engine(vault.path(), StateDir::at(state.path()).unwrap(), None).unwrap();
        Rig {
            vault,
            state,
            engine,
        }
    }

    /// Shuts the engine down and hands back the vault and state directories.
    fn stop(self) -> (TempDir, TempDir) {
        self.engine.shutdown().unwrap();
        (self.vault, self.state)
    }

    fn write(&self, name: &str, content: &str) {
        fs::write(self.vault.path().join(name), content).unwrap();
    }

    fn connected_to(&self, device_id: &str) -> bool {
        let status = self.engine.status().unwrap();
        status
            .peers
            .iter()
            .any(|peer| peer.id == device_id && peer.connected)
    }

    fn any_peer_connected(&self) -> bool {
        let status = self.engine.status().unwrap();
        status.peers.iter().any(|peer| peer.connected)
    }

    /// A ticket whose only address is the loopback address on the engine's IPv4 port.
    fn loopback_ticket(&self) -> Ticket {
        let mut ticket: Ticket = self.engine.invite().unwrap().parse().unwrap();
        assert!(!ticket.addrs.is_empty(), "the ticket has no address");
        let port = ticket
            .addrs
            .iter()
            .find(|addr| addr.is_ipv4())
            .unwrap_or(&ticket.addrs[0])
            .port();
        ticket.addrs = vec![SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)];
        ticket
    }
}

fn wait_until(limit: Duration, what: &str, mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + limit;
    while !condition() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(POLL);
    }
}

fn has_content(vault: &Path, name: &str, content: &str) -> bool {
    fs::read_to_string(vault.join(name)).is_ok_and(|found| found == content)
}

fn join_pair() -> (Rig, Rig, Ticket) {
    let a = Rig::start();
    let ticket = a.loopback_ticket();
    let b = Rig::start();
    b.engine.join(&ticket.to_string()).unwrap();
    let a_id = to_hex(&ticket.device_id);
    wait_until(CONNECT_LIMIT, "B to show A connected", || {
        b.connected_to(&a_id)
    });
    (a, b, ticket)
}

#[test]
fn should_sync_two_engines_over_loopback() {
    let (a, b, _) = join_pair();
    a.write("from-a.md", "written in a\n");
    wait_until(FILE_LIMIT, "from-a.md in B", || {
        has_content(b.vault.path(), "from-a.md", "written in a\n")
    });
    b.write("from-b.md", "written in b\n");
    wait_until(FILE_LIMIT, "from-b.md in A", || {
        has_content(a.vault.path(), "from-b.md", "written in b\n")
    });
    b.write("from-a.md", "edited in b\n");
    wait_until(FILE_LIMIT, "the edit in A", || {
        has_content(a.vault.path(), "from-a.md", "edited in b\n")
    });
    for (_, state_dir) in [a.stop(), b.stop()] {
        let state = StateDir::at(state_dir.path()).unwrap();
        assert!(state.lock().is_ok(), "the lock is still held");
    }
}

#[test]
fn should_refuse_a_peer_with_another_secret() {
    let a = Rig::start();
    let ticket = a.loopback_ticket();
    let a_id = to_hex(&ticket.device_id);
    let c = Rig::start();
    let entry = PeerEntry {
        id: a_id.clone(),
        addrs: ticket.addrs.iter().map(ToString::to_string).collect(),
    };
    StateDir::at(c.state.path())
        .unwrap()
        .add_peer(entry)
        .unwrap();
    a.write("only-a.md", "a\n");
    c.write("only-c.md", "c\n");
    // Positive control: A accepts a peer of its own group while C, which dials A on its ticks (assumed, not
    // observed), stays refused.
    let d = Rig::start();
    d.engine.join(&ticket.to_string()).unwrap();
    wait_until(CONNECT_LIMIT, "D to show A connected", || {
        d.connected_to(&a_id)
    });
    assert_group_stays_apart(&a, &c, &d);
}

/// For 15 s, A never shows C connected, C shows nobody, and C's note and A's note never cross.
fn assert_group_stays_apart(a: &Rig, c: &Rig, d: &Rig) {
    let c_id = c.engine.status().unwrap().device_id;
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        assert!(!a.connected_to(&c_id), "A shows C connected");
        assert!(!c.any_peer_connected(), "C shows a peer connected");
        assert!(!a.vault.path().join("only-c.md").exists());
        assert!(!d.vault.path().join("only-c.md").exists());
        assert!(!c.vault.path().join("only-a.md").exists());
        std::thread::sleep(POLL);
    }
}

#[test]
fn should_keep_the_same_device_id_across_restarts() {
    let rig = Rig::start();
    let first = rig.engine.status().unwrap().device_id;
    let (vault, state) = rig.stop();
    let again = Rig::start_in(vault, state);
    assert_eq!(again.engine.status().unwrap().device_id, first);
}

#[test]
fn should_drop_a_silent_connection_and_keep_serving() {
    let a = Rig::start();
    let ticket = a.loopback_ticket();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (endpoint, connection) = runtime.block_on(connect_silently(&ticket));
    let started = Instant::now();
    let closing = runtime.spawn(async move { (connection.closed().await, started.elapsed()) });
    let b = Rig::start();
    b.engine.join(&ticket.to_string()).unwrap();
    a.write("served.md", "still served\n");
    wait_until(CONNECT_LIMIT, "A to serve B beside the silent peer", || {
        has_content(b.vault.path(), "served.md", "still served\n")
    });
    let latest = Duration::from_secs(HANDSHAKE_SECS + 2).saturating_sub(started.elapsed());
    let closed = runtime.block_on(async { tokio::time::timeout(latest, closing).await });
    let (reason, after) = closed
        .expect("the silent connection was not dropped")
        .unwrap();
    assert_dropped_as_refused(&reason, after);
    runtime.block_on(endpoint.close());
}

/// The silent connection ended by the 10 s deadline, not earlier, with the refusal code.
fn assert_dropped_as_refused(reason: &ConnectionError, after: Duration) {
    let earliest = Duration::from_secs(HANDSHAKE_SECS - 1);
    assert!(after >= earliest, "dropped after {after:?}, too early");
    let refused = VarInt::from_u32(CLOSE_REFUSED);
    assert!(
        matches!(reason, ConnectionError::ApplicationClosed(close) if close.error_code == refused),
        "closed with {reason:?}"
    );
}

async fn connect_silently(ticket: &Ticket) -> (Endpoint, iroh::endpoint::Connection) {
    let endpoint = Endpoint::builder(presets::Minimal)
        .portmapper_config(iroh::endpoint::PortmapperConfig::Disabled)
        .secret_key(SecretKey::generate())
        .bind()
        .await
        .unwrap();
    let id = EndpointId::from_bytes(&ticket.device_id).unwrap();
    let mut target = EndpointAddr::new(id);
    for addr in &ticket.addrs {
        target = target.with_ip_addr(*addr);
    }
    let connection = endpoint.connect(target, ALPN).await.unwrap();
    (endpoint, connection)
}

#[test]
#[ignore = "needs outbound HTTPS and UDP to the public iroh infrastructure"]
fn should_connect_in_internet_mode_by_device_id_alone() {
    let a = Rig::start_internet();
    let mut ticket: Ticket = a.engine.invite().unwrap().parse().unwrap();
    ticket.addrs.clear();
    let a_id = to_hex(&ticket.device_id);
    let b = Rig::start_internet();
    b.engine.join(&ticket.to_string()).unwrap();
    wait_until(Duration::from_secs(60), "B to show A connected", || {
        b.connected_to(&a_id)
    });
}

#[test]
#[ignore = "needs the public internet; run with --ignored to record the real status"]
fn should_come_online_in_internet_mode() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let endpoint = Endpoint::builder(presets::N0).bind().await.unwrap();
        let online = tokio::time::timeout(Duration::from_secs(30), endpoint.online()).await;
        assert!(
            online.is_ok(),
            "the N0 endpoint did not come online in 30 s"
        );
    });
}
