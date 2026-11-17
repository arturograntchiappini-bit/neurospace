//! The network adapter: an iroh endpoint that accepts and dials peers and runs one sync session per
//! connection (spec section 8.8).

use std::collections::HashMap;
use std::future::Future;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use iroh::endpoint::{Connection, PortmapperConfig, VarInt, presets};
use iroh::protocol::{AcceptError, ProtocolHandler, Router};
use iroh::{Endpoint, EndpointAddr, EndpointId, SecretKey};
use iroh_mdns_address_lookup::MdnsAddressLookup;
use tokio::runtime::{Handle, Runtime};
use tokio::task::AbortHandle;

use crate::auth::group_tag;
use crate::engine::{Command, EngineEvent, EngineHandle, Network};
use crate::error::{Result, SyncError};
use crate::protocol::ALPN;
use crate::replica::SessionId;
use crate::session::{Role, SessionEvent, SessionParams, run_session};
use crate::state::{DeviceId, NetConfig, NetMode, PeerEntry, Secret, StateDir, from_hex32};

/// How long a dial may take before it is given up.
pub const DIAL_TIMEOUT: Duration = Duration::from_secs(15);
/// How long after a connection is accepted or dialled its session may take to report `Opened`.
pub const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// How many accepted connections may be in the handshake at once; one more is closed unread.
pub const MAX_PENDING_HANDSHAKES: usize = 16;

/// How long a finished session waits for the peer to close first, so its last bytes are not cut off.
const CLOSE_GRACE: Duration = Duration::from_secs(1);
/// The router gets this long to stop; then the runtime gets `RUNTIME_GRACE`.
const ROUTER_GRACE: Duration = Duration::from_secs(2);
const RUNTIME_GRACE: Duration = Duration::from_secs(1);
const WORKER_THREADS: usize = 2;
const CLOSE_REFUSED: VarInt = VarInt::from_u32(1);
const CLOSE_DONE: VarInt = VarInt::from_u32(0);

/// A running session task: the peer it dials (for dials) and the handle that aborts it.
struct Task {
    dialing: Option<DeviceId>,
    abort: AbortHandle,
}

/// What the accept path and the dial path share; one lock keeps a group change and a new session apart.
struct Group {
    secret: Option<Secret>,
    tasks: HashMap<SessionId, Task>,
}

struct Shared {
    own_id: DeviceId,
    commands: Sender<Command>,
    next_session: AtomicU64,
    pending: Arc<AtomicUsize>,
    group: Mutex<Group>,
}

impl Shared {
    fn new(own_id: DeviceId, commands: Sender<Command>) -> Arc<Shared> {
        Arc::new(Shared {
            own_id,
            commands,
            next_session: AtomicU64::new(1),
            pending: Arc::new(AtomicUsize::new(0)),
            group: Mutex::new(Group {
                secret: None,
                tasks: HashMap::new(),
            }),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Group> {
        self.group.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn next_id(&self) -> SessionId {
        self.next_session.fetch_add(1, Ordering::SeqCst)
    }

    /// Stores the secret for new sessions. When it replaces a different one, every task is removed and its
    /// handle returned: the caller aborts them outside the lock, because an abort can run a task's drop inline.
    fn install_secret(&self, secret: Secret) -> Vec<AbortHandle> {
        let mut group = self.lock();
        let changed = group.secret.is_some_and(|old| old != secret);
        group.secret = Some(secret);
        if !changed {
            return Vec::new();
        }
        group.tasks.drain().map(|(_, task)| task.abort).collect()
    }

    fn params(&self, role: Role, secret: Secret, peer_id: DeviceId) -> SessionParams {
        SessionParams {
            role,
            secret,
            own_id: self.own_id,
            peer_id,
        }
    }

    /// Starts a session task for a connection a peer opened, or closes the connection when it cannot be served.
    fn accept(self: &Arc<Self>, conn: Connection) {
        let Some(pending) = PendingGuard::acquire(&self.pending, MAX_PENDING_HANDSHAKES) else {
            return conn.close(CLOSE_REFUSED, b"too many handshakes");
        };
        let mut group = self.lock();
        let Some(secret) = group.secret else {
            return conn.close(CLOSE_REFUSED, b"not ready");
        };
        let id = self.next_id();
        let params = self.params(Role::Acceptor, secret, *conn.remote_id().as_bytes());
        let task = SessionTask::new(self, id, params, Some(pending));
        let abort = tokio::spawn(task.run(std::future::ready(Some(conn)))).abort_handle();
        group.tasks.insert(
            id,
            Task {
                dialing: None,
                abort,
            },
        );
    }

    /// Starts a task that dials `target`, unless one is already dialling that peer.
    fn dial(self: &Arc<Self>, runtime: &Handle, endpoint: &Endpoint, target: EndpointAddr) {
        let peer = *target.id.as_bytes();
        let mut group = self.lock();
        let Some(secret) = group.secret else { return };
        if group.tasks.values().any(|task| task.dialing == Some(peer)) {
            return;
        }
        let id = self.next_id();
        let params = self.params(Role::Dialer, secret, peer);
        let task = SessionTask::new(self, id, params, None);
        let connect = connect_to(endpoint.clone(), target);
        let abort = runtime.spawn(task.run(connect)).abort_handle();
        group.tasks.insert(
            id,
            Task {
                dialing: Some(peer),
                abort,
            },
        );
    }

    fn take_task(&self, session: SessionId) -> Option<Task> {
        self.lock().tasks.remove(&session)
    }
}

async fn connect_to(endpoint: Endpoint, target: EndpointAddr) -> Option<Connection> {
    tokio::time::timeout(DIAL_TIMEOUT, endpoint.connect(target, ALPN))
        .await
        .ok()?
        .ok()
}

/// One accepted connection in its handshake; frees its slot on drop.
struct PendingGuard(Arc<AtomicUsize>);

impl PendingGuard {
    fn acquire(counter: &Arc<AtomicUsize>, max: usize) -> Option<PendingGuard> {
        let taken = counter.try_update(Ordering::SeqCst, Ordering::SeqCst, |count| {
            (count < max).then_some(count + 1)
        });
        taken.ok().map(|_| PendingGuard(counter.clone()))
    }
}

impl Drop for PendingGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Removes a session's entry when its task ends or is aborted.
struct Registered {
    shared: Arc<Shared>,
    id: SessionId,
}

impl Drop for Registered {
    fn drop(&mut self) {
        self.shared.take_task(self.id);
    }
}

/// Runs `session`; gives up (`None`) when `limit` passes before `opened` is set.
async fn until_handshake_deadline<F: Future>(
    opened: &AtomicBool,
    limit: Duration,
    session: F,
) -> Option<F::Output> {
    let expired = async {
        tokio::time::sleep(limit).await;
        if opened.load(Ordering::SeqCst) {
            std::future::pending::<()>().await;
        }
    };
    tokio::select! {
        output = session => Some(output),
        () = expired => None,
    }
}

/// Everything one session task owns.
struct SessionTask {
    shared: Arc<Shared>,
    id: SessionId,
    params: SessionParams,
    pending: Option<PendingGuard>,
}

impl SessionTask {
    fn new(
        shared: &Arc<Shared>,
        id: SessionId,
        params: SessionParams,
        pending: Option<PendingGuard>,
    ) -> SessionTask {
        SessionTask {
            shared: shared.clone(),
            id,
            params,
            pending,
        }
    }

    /// Connects, then relays until the session ends or the handshake deadline passes.
    async fn run(self, connect: impl Future<Output = Option<Connection>>) {
        let _registered = Registered {
            shared: self.shared.clone(),
            id: self.id,
        };
        let Some(conn) = connect.await else { return };
        let opened = Arc::new(AtomicBool::new(false));
        let relay = self.relay(&conn, opened.clone());
        if until_handshake_deadline(&opened, HANDSHAKE_TIMEOUT, relay)
            .await
            .is_some()
        {
            let _ = tokio::time::timeout(CLOSE_GRACE, conn.closed()).await;
        }
        conn.close(CLOSE_DONE, b"done");
    }

    /// Opens or accepts the stream and runs the session on it. Every event is stamped with the tag of the
    /// secret this session authenticates with. A failing session is not an engine error: the result is dropped.
    async fn relay(self, conn: &Connection, opened: Arc<AtomicBool>) {
        let SessionTask {
            shared,
            id,
            params,
            mut pending,
        } = self;
        let streams = match params.role {
            Role::Dialer => conn.open_bi().await,
            Role::Acceptor => conn.accept_bi().await,
        };
        let Ok((send, recv)) = streams else { return };
        let group = group_tag(&params.secret);
        let commands = shared.commands.clone();
        let _ = run_session(recv, send, params, move |event| {
            if matches!(event, SessionEvent::Opened { .. }) {
                opened.store(true, Ordering::SeqCst);
                pending.take();
            }
            let _ = commands.send(Command::Session {
                session: id,
                group,
                event,
            });
        })
        .await;
    }
}

struct Acceptor {
    shared: Arc<Shared>,
}

impl std::fmt::Debug for Acceptor {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        out.debug_struct("Acceptor").finish_non_exhaustive()
    }
}

impl ProtocolHandler for Acceptor {
    async fn accept(&self, conn: Connection) -> std::result::Result<(), AcceptError> {
        self.shared.accept(conn);
        Ok(())
    }
}

fn net_error(error: impl std::fmt::Display) -> SyncError {
    SyncError::Net(error.to_string())
}

fn load_or_create_key(state: &StateDir) -> Result<SecretKey> {
    if let Some(bytes) = state.device_key()? {
        return Ok(SecretKey::from_bytes(&bytes));
    }
    let key = SecretKey::generate();
    state.save_device_key(&key.to_bytes())?;
    Ok(key)
}

/// Makes sure the device key, the device id and the group secret exist, and returns the device id.
/// The network takes its key from the same place, so an invitation and a running engine always agree.
///
/// # Errors
/// Failures of the state directory.
pub fn ensure_identity(state: &StateDir) -> Result<DeviceId> {
    let id = *load_or_create_key(state)?.public().as_bytes();
    state.save_device_id(&id)?;
    state.ensure_secret()?;
    Ok(id)
}

async fn bind_endpoint(key: SecretKey, config: &NetConfig) -> Result<Endpoint> {
    let builder = match config.mode {
        NetMode::Lan => {
            Endpoint::builder(presets::Minimal).portmapper_config(PortmapperConfig::Disabled)
        }
        NetMode::Internet => Endpoint::builder(presets::N0),
    };
    let mut builder = builder.secret_key(key).alpns(vec![ALPN.to_vec()]);
    if config.mdns {
        builder = builder.address_lookup(MdnsAddressLookup::builder());
    }
    if let Some(port) = config.bind_port {
        let any_interface = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
        builder = builder
            .clear_ip_transports()
            .bind_addr(any_interface)
            .map_err(net_error)?;
    }
    builder.bind().await.map_err(net_error)
}

async fn serve(
    key: SecretKey,
    config: &NetConfig,
    shared: Arc<Shared>,
) -> Result<(Endpoint, Router)> {
    let endpoint = bind_endpoint(key, config).await?;
    let router = Router::builder(endpoint.clone())
        .accept(ALPN, Acceptor { shared })
        .spawn();
    Ok((endpoint, router))
}

/// A peer's id and the address hints that parse; `None` when the id is not a device id.
fn dial_target(peer: &PeerEntry) -> Option<EndpointAddr> {
    let id = EndpointId::from_bytes(&from_hex32(&peer.id)?).ok()?;
    let hints = peer
        .addrs
        .iter()
        .filter_map(|text| text.parse::<SocketAddr>().ok());
    Some(hints.fold(EndpointAddr::new(id), EndpointAddr::with_ip_addr))
}

/// The iroh network of one engine: owns a tokio runtime, the endpoint and the accepting router.
/// `start`, `shutdown` and `Drop` block on the runtime, so the host calls them from a plain thread: `start`
/// refuses to run on a tokio thread, and there `shutdown` and `Drop` only detach the runtime.
pub struct IrohNetwork {
    runtime: Option<Runtime>,
    endpoint: Endpoint,
    router: Router,
    shared: Arc<Shared>,
}

impl IrohNetwork {
    /// Loads or creates the device key, binds an endpoint for the configured mode and starts accepting.
    ///
    /// # Errors
    /// State failures, or `Net` with the text of the iroh error, or when called on a tokio thread.
    pub fn start(commands: Sender<Command>, state: &StateDir) -> Result<IrohNetwork> {
        if Handle::try_current().is_ok() {
            return Err(SyncError::Net(
                "the network cannot be started on a tokio thread".to_string(),
            ));
        }
        let key = load_or_create_key(state)?;
        let config = state.config()?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(WORKER_THREADS)
            .enable_all()
            .build()
            .map_err(net_error)?;
        let shared = Shared::new(*key.public().as_bytes(), commands);
        let (endpoint, router) = runtime.block_on(serve(key, &config, shared.clone()))?;
        Ok(IrohNetwork {
            runtime: Some(runtime),
            endpoint,
            router,
            shared,
        })
    }
}

impl Network for IrohNetwork {
    fn device_id(&self) -> DeviceId {
        self.shared.own_id
    }

    fn addresses(&self) -> Vec<SocketAddr> {
        self.endpoint.addr().ip_addrs().copied().collect()
    }

    fn refresh(&mut self, secret: Secret, peers: &[PeerEntry], connected: &[DeviceId]) {
        for stale in self.shared.install_secret(secret) {
            stale.abort();
        }
        let Some(runtime) = self.runtime.as_ref().map(Runtime::handle) else {
            return;
        };
        for target in peers.iter().filter_map(dial_target) {
            let peer = *target.id.as_bytes();
            if peer != self.shared.own_id && !connected.contains(&peer) {
                self.shared.dial(runtime, &self.endpoint, target);
            }
        }
    }

    fn close_session(&mut self, session: SessionId) {
        if let Some(task) = self.shared.take_task(session) {
            task.abort.abort();
        }
    }

    fn shutdown(&mut self) {
        let Some(runtime) = self.runtime.take() else {
            return;
        };
        if Handle::try_current().is_ok() {
            return runtime.shutdown_background();
        }
        let stopping = async { tokio::time::timeout(ROUTER_GRACE, self.router.shutdown()).await };
        let _ = runtime.block_on(stopping);
        runtime.shutdown_timeout(RUNTIME_GRACE);
    }
}

impl Drop for IrohNetwork {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The one call a host needs: an engine with the real network. The network starts on the calling thread,
/// which must not be a tokio thread.
///
/// # Errors
/// `AlreadyRunning`, and whatever the network, the vault or the replica report.
pub fn start_engine(
    vault_root: &Path,
    state: StateDir,
    events: Option<Sender<EngineEvent>>,
) -> Result<EngineHandle> {
    EngineHandle::start(vault_root, state, events, |commands, state| {
        Ok(Box::new(IrohNetwork::start(commands, state)?))
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use super::*;

    const SHORT: Duration = Duration::from_millis(100);

    #[test]
    fn should_refuse_a_handshake_over_the_cap_and_free_the_slot_on_drop() {
        let counter = Arc::new(AtomicUsize::new(0));
        let held: Vec<_> = (0..3)
            .map(|_| PendingGuard::acquire(&counter, 3).unwrap())
            .collect();
        assert!(PendingGuard::acquire(&counter, 3).is_none());
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        drop(held);
        assert_eq!(counter.load(Ordering::SeqCst), 0);
        assert!(PendingGuard::acquire(&counter, 3).is_some());
    }

    #[tokio::test]
    async fn should_end_a_session_that_never_opens_within_the_deadline() {
        let opened = AtomicBool::new(false);
        let never = std::future::pending::<()>();
        assert!(
            until_handshake_deadline(&opened, SHORT, never)
                .await
                .is_none()
        );
    }

    #[tokio::test]
    async fn should_let_an_opened_session_outlive_the_deadline() {
        let opened = AtomicBool::new(true);
        let long = async {
            tokio::time::sleep(SHORT * 3).await;
            7
        };
        assert_eq!(
            until_handshake_deadline(&opened, SHORT, long).await,
            Some(7)
        );
    }

    #[tokio::test]
    async fn should_abort_every_task_only_when_the_secret_changes() {
        let (commands, _inbox) = mpsc::channel();
        let shared = Shared::new([1; 32], commands);
        let task = tokio::spawn(std::future::pending::<()>());
        shared.lock().tasks.insert(
            shared.next_id(),
            Task {
                dialing: None,
                abort: task.abort_handle(),
            },
        );
        assert!(shared.install_secret([5; 32]).is_empty());
        assert!(shared.install_secret([5; 32]).is_empty());
        assert_eq!(shared.install_secret([6; 32]).len(), 1);
        assert!(shared.install_secret([7; 32]).is_empty());
    }

    #[test]
    fn should_number_sessions_from_one_without_reuse() {
        let (commands, _inbox) = mpsc::channel();
        let shared = Shared::new([1; 32], commands);
        assert_eq!(
            (shared.next_id(), shared.next_id(), shared.next_id()),
            (1, 2, 3)
        );
    }

    fn lan_state(config: &str) -> (tempfile::TempDir, StateDir) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("config.json"), config).unwrap();
        let state = StateDir::at(dir.path()).unwrap();
        (dir, state)
    }

    #[test]
    fn should_not_panic_when_dropped_on_a_tokio_thread() {
        let (_dir, state) = lan_state(r#"{"mode":"lan","mdns":false}"#);
        let (commands, _inbox) = mpsc::channel();
        let network = IrohNetwork::start(commands, &state).unwrap();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async move { drop(network) });
    }

    #[test]
    fn should_refuse_to_start_on_a_tokio_thread() {
        let (_dir, state) = lan_state(r#"{"mode":"lan","mdns":false}"#);
        let (commands, _inbox) = mpsc::channel();
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let started = runtime.block_on(async { IrohNetwork::start(commands, &state) });
        assert!(matches!(started, Err(SyncError::Net(_))));
    }

    #[test]
    fn should_create_the_identity_once_and_start_the_network_with_it() {
        let (_dir, state) = lan_state(r#"{"mode":"lan","mdns":false}"#);
        let id = ensure_identity(&state).unwrap();
        assert_eq!(ensure_identity(&state).unwrap(), id);
        assert_eq!(state.device_id().unwrap(), Some(id));
        assert!(state.secret().unwrap().is_some());
        let (commands, _inbox) = mpsc::channel();
        let network = IrohNetwork::start(commands, &state).unwrap();
        assert_eq!(network.device_id(), id);
    }

    #[test]
    fn should_listen_on_the_configured_port() {
        let probe = std::net::UdpSocket::bind("0.0.0.0:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);
        let config = format!(r#"{{"mode":"lan","mdns":false,"bind_port":{port}}}"#);
        let (_dir, state) = lan_state(&config);
        let (commands, _inbox) = mpsc::channel();
        let network = IrohNetwork::start(commands, &state).unwrap();
        let addresses = network.addresses();
        assert!(!addresses.is_empty());
        assert!(addresses.iter().all(|address| address.port() == port));
    }
}
