use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, ErrorKind, Write};
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::error::{Result, SyncError};

/// The 32-byte group secret shared by every replica of a sync group.
pub type Secret = [u8; 32];
/// A device's public id (the public half of its transport key).
pub type DeviceId = [u8; 32];

const APP_DIR: &str = "neurospace";
/// The most peers `peers.json` holds; a new peer beyond it is dropped and the oldest stay.
pub const MAX_STORED_PEERS: usize = 256;
const GROUP_FILES: [&str; 9] = [
    "secret",
    "peers.json",
    "doc.snap",
    "doc.log",
    "base.json",
    "base.version",
    "fresh-join",
    "command.pending",
    "status.json",
];

/// Fills an array with operating-system randomness.
pub fn random_bytes<const N: usize>() -> Result<[u8; N]> {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).map_err(|error| SyncError::Io {
        path: PathBuf::from("getrandom"),
        source: io::Error::other(error.to_string()),
    })?;
    Ok(bytes)
}

/// Lowercase hexadecimal encoding.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Decodes hexadecimal text (either case, even length) into bytes.
pub(crate) fn from_hex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) || !text.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    text.as_bytes()
        .chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// Decodes exactly 64 hexadecimal characters (either case) into 32 bytes.
pub fn from_hex32(text: &str) -> Option<[u8; 32]> {
    from_hex(text)?.try_into().ok()
}

/// A known peer: its device id (64 hex characters) and address hints (`ip:port`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerEntry {
    /// The peer's device id, 64 hexadecimal characters.
    pub id: String,
    /// Addresses the peer was last reachable at.
    #[serde(default)]
    pub addrs: Vec<String>,
}

/// How the engine reaches other devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetMode {
    /// Public discovery and relay servers, plus the local network.
    Internet,
    /// Local network only: no contact with external servers.
    Lan,
}

/// Network settings read from `config.json`; every field is optional in the file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NetConfig {
    /// Where the engine looks for peers.
    pub mode: NetMode,
    /// A fixed local port, or `None` for any free port.
    pub bind_port: Option<u16>,
    /// Whether to announce and discover peers with mDNS.
    pub mdns: bool,
}

impl Default for NetConfig {
    fn default() -> Self {
        Self {
            mode: NetMode::Internet,
            bind_port: None,
            mdns: true,
        }
    }
}

/// A command for the running engine, left in `command.pending`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PendingCommand {
    /// Join the group described by this ticket.
    Join(String),
    /// Leave the group (the flag is always `true`).
    Leave(bool),
}

// A join ticket contains the group secret, so it never reaches logs.
impl std::fmt::Debug for PendingCommand {
    fn fmt(&self, out: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Join(_) => out.debug_tuple("Join").field(&"<redacted>").finish(),
            Self::Leave(flag) => out.debug_tuple("Leave").field(flag).finish(),
        }
    }
}

/// A peer as shown in the status file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PeerStatus {
    /// The peer's device id, 64 hexadecimal characters.
    pub id: String,
    /// Whether a connection is currently open.
    pub connected: bool,
}

/// A note the engine cannot sync, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BlockedNote {
    /// The note's path or file name.
    pub path: String,
    /// Why it is blocked.
    pub reason: String,
}

/// What the engine writes to `status.json` on every tick.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Status {
    /// Process id of the engine.
    pub pid: u32,
    /// This device's id, 64 hexadecimal characters.
    pub device_id: String,
    /// Addresses this device listens on.
    pub addresses: Vec<String>,
    /// Known peers and their connection state.
    pub peers: Vec<PeerStatus>,
    /// Number of notes in the sync document.
    pub notes: usize,
    /// Notes that cannot be synced right now.
    pub blocked: Vec<BlockedNote>,
    /// Entries of the sync document that a peer made up or garbled, skipped as malformed.
    #[serde(default)]
    pub rejected_entries: usize,
    /// The relay the network reached, if any: without one, `internet` mode cannot find peers by device id.
    #[serde(default)]
    pub relay: Option<String>,
    /// The most recent error, if any.
    pub last_error: Option<String>,
    /// Time of this update, in seconds since the Unix epoch.
    pub updated_unix: u64,
}

fn non_empty(env: &dyn Fn(&str) -> Option<String>, name: &str) -> Option<String> {
    env(name).filter(|value| !value.is_empty())
}

/// Chooses the base state directory from environment lookups (pure twin of the real environment).
pub fn base_dir_from(windows: bool, env: &dyn Fn(&str) -> Option<String>) -> Result<PathBuf> {
    if let Some(dir) = non_empty(env, "NS_STATE_DIR") {
        return Ok(PathBuf::from(dir));
    }
    let base = if windows {
        non_empty(env, "LOCALAPPDATA").map(PathBuf::from)
    } else {
        non_empty(env, "XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                non_empty(env, "HOME").map(|home| PathBuf::from(home).join(".local").join("share"))
            })
    };
    base.map(|dir| dir.join(APP_DIR))
        .ok_or(SyncError::NoStateDir)
}

/// Directory name of a vault's state: 16 hex characters of the path hash (case-folded on Windows).
pub fn vault_dir_name(canonical_vault_path: &str, windows: bool) -> String {
    let text = if windows {
        canonical_vault_path.to_lowercase()
    } else {
        canonical_vault_path.to_string()
    };
    to_hex(&blake3::hash(text.as_bytes()).as_bytes()[..8])
}

pub(crate) fn io_error(path: &Path) -> impl FnOnce(io::Error) -> SyncError + '_ {
    move |source| SyncError::Io {
        path: path.to_path_buf(),
        source,
    }
}

pub(crate) fn corrupt(path: &Path, reason: impl ToString) -> SyncError {
    SyncError::CorruptState {
        path: path.to_path_buf(),
        reason: reason.to_string(),
    }
}

pub(crate) fn read_optional(path: &Path) -> Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error(path)(error)),
    }
}

pub(crate) fn remove_if_exists(path: &Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(io_error(path)(error)),
    }
}

/// Options for every file in the state directory: readable by the owner only on Unix.
#[cfg(unix)]
pub(crate) fn private_options() -> OpenOptions {
    use std::os::unix::fs::OpenOptionsExt;
    let mut options = OpenOptions::new();
    options.mode(0o600);
    options
}

#[cfg(not(unix))]
pub(crate) fn private_options() -> OpenOptions {
    OpenOptions::new()
}

/// Creates a directory and its missing parents, accessible to the owner only on Unix.
#[cfg(unix)]
fn create_private_dirs(dir: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)
}

#[cfg(not(unix))]
fn create_private_dirs(dir: &Path) -> io::Result<()> {
    std::fs::create_dir_all(dir)
}

fn write_temporary(tmp: &Path, bytes: &[u8]) -> Result<()> {
    // A stale file would keep its older, possibly wider, mode when reopened.
    let _ = std::fs::remove_file(tmp);
    let written = private_options()
        .write(true)
        .create(true)
        .truncate(true)
        .open(tmp)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()));
    written.map_err(|source| {
        let _ = std::fs::remove_file(tmp);
        io_error(tmp)(source)
    })
}

pub(crate) fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    write_temporary(&tmp, bytes)?;
    std::fs::rename(&tmp, path).map_err(|source| {
        let _ = std::fs::remove_file(&tmp);
        io_error(path)(source)
    })
}

/// Describes a JSON failure by kind and position only: the message may quote the input.
fn json_corrupt(path: &Path, error: &serde_json::Error) -> SyncError {
    let reason = format!(
        "{:?} error at line {} column {}",
        error.classify(),
        error.line(),
        error.column()
    );
    corrupt(path, reason)
}

pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(None);
    };
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|error| json_corrupt(path, &error))
}

pub(crate) fn write_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let bytes = serde_json::to_vec(value).map_err(|error| SyncError::Doc(error.to_string()))?;
    write_atomic(path, &bytes)
}

fn read_hex32(path: &Path) -> Result<Option<[u8; 32]>> {
    let Some(bytes) = read_optional(path)? else {
        return Ok(None);
    };
    let text = String::from_utf8_lossy(&bytes);
    from_hex32(text.trim())
        .map(Some)
        .ok_or_else(|| corrupt(path, "expected 64 hexadecimal characters"))
}

/// The per-vault directory that holds all sync state outside the vault.
#[derive(Debug, Clone)]
pub struct StateDir {
    dir: PathBuf,
}

/// Proof that this process is the only engine of a vault; the lock is released on drop.
#[derive(Debug)]
pub struct EngineLock {
    _file: File,
}

impl StateDir {
    /// The state directory of a vault, under the base directory of the real environment.
    pub fn for_vault(vault_root: &Path) -> Result<StateDir> {
        let base = base_dir_from(cfg!(windows), &|name| std::env::var(name).ok())?;
        Self::for_vault_in(&base, vault_root)
    }

    /// The state directory of a vault under an explicit base directory.
    pub fn for_vault_in(base: &Path, vault_root: &Path) -> Result<StateDir> {
        let canonical = std::fs::canonicalize(vault_root).map_err(io_error(vault_root))?;
        let canonical = canonical.to_string_lossy();
        let name = vault_dir_name(&canonical, cfg!(windows));
        let state = Self::at(base.join("vaults").join(name))?;
        let marker = state.file("vault-path.txt");
        if !marker.exists() {
            write_atomic(&marker, canonical.as_bytes())?;
        }
        Ok(state)
    }

    /// Exactly this directory, created if missing.
    pub fn at(dir: impl Into<PathBuf>) -> Result<StateDir> {
        let dir = dir.into();
        create_private_dirs(&dir).map_err(io_error(&dir))?;
        Ok(StateDir { dir })
    }

    /// The directory itself.
    pub fn path(&self) -> &Path {
        &self.dir
    }

    /// The path of a file inside the directory.
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Takes the engine lock, or fails with `AlreadyRunning` when another process holds it.
    pub fn lock(&self) -> Result<EngineLock> {
        let path = self.file("engine.lock");
        let file = private_options()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)
            .map_err(io_error(&path))?;
        match file.try_lock() {
            Ok(()) => Ok(EngineLock { _file: file }),
            Err(TryLockError::WouldBlock) => Err(SyncError::AlreadyRunning),
            Err(TryLockError::Error(source)) => Err(SyncError::Io { path, source }),
        }
    }

    /// The device's secret key, if one was saved.
    pub fn device_key(&self) -> Result<Option<[u8; 32]>> {
        read_hex32(&self.file("device.key"))
    }

    /// Saves the device's secret key.
    pub fn save_device_key(&self, key: &[u8; 32]) -> Result<()> {
        write_atomic(&self.file("device.key"), to_hex(key).as_bytes())
    }

    /// The device's public id, as last written by the engine.
    pub fn device_id(&self) -> Result<Option<DeviceId>> {
        read_hex32(&self.file("device.id"))
    }

    /// Saves the device's public id.
    pub fn save_device_id(&self, id: &DeviceId) -> Result<()> {
        write_atomic(&self.file("device.id"), to_hex(id).as_bytes())
    }

    /// The group secret; its presence means sync is on.
    pub fn secret(&self) -> Result<Option<Secret>> {
        read_hex32(&self.file("secret"))
    }

    /// Saves the group secret.
    pub fn save_secret(&self, secret: &Secret) -> Result<()> {
        write_atomic(&self.file("secret"), to_hex(secret).as_bytes())
    }

    /// The group secret, created at random when missing.
    pub fn ensure_secret(&self) -> Result<Secret> {
        if let Some(secret) = self.secret()? {
            return Ok(secret);
        }
        let secret = random_bytes()?;
        self.save_secret(&secret)?;
        Ok(secret)
    }

    /// Known peers in order of first insertion; empty when none were saved.
    pub fn peers(&self) -> Result<Vec<PeerEntry>> {
        Ok(read_json(&self.file("peers.json"))?.unwrap_or_default())
    }

    /// Replaces the list of known peers.
    pub fn save_peers(&self, peers: &[PeerEntry]) -> Result<()> {
        write_json(&self.file("peers.json"), &peers)
    }

    /// Adds a peer, or merges new addresses into a known one. True when the id is new and was stored;
    /// at `MAX_STORED_PEERS` a new id is dropped.
    pub fn add_peer(&self, peer: PeerEntry) -> Result<bool> {
        let mut peers = self.peers()?;
        let Some(known) = peers.iter_mut().find(|known| known.id == peer.id) else {
            if peers.len() >= MAX_STORED_PEERS {
                return Ok(false);
            }
            peers.push(peer);
            self.save_peers(&peers)?;
            return Ok(true);
        };
        let before = known.addrs.len();
        for addr in peer.addrs {
            if !known.addrs.contains(&addr) {
                known.addrs.push(addr);
            }
        }
        if known.addrs.len() != before {
            self.save_peers(&peers)?;
        }
        Ok(false)
    }

    /// The network settings, with defaults for a missing file or missing fields.
    pub fn config(&self) -> Result<NetConfig> {
        Ok(read_json(&self.file("config.json"))?.unwrap_or_default())
    }

    /// Replaces the network settings.
    pub fn save_config(&self, config: &NetConfig) -> Result<()> {
        write_json(&self.file("config.json"), config)
    }

    /// Whether this replica joined a group and has not reconciled since.
    pub fn fresh_join(&self) -> bool {
        self.file("fresh-join").exists()
    }

    /// Sets or clears the fresh-join marker.
    pub fn set_fresh_join(&self, on: bool) -> Result<()> {
        let marker = self.file("fresh-join");
        if on {
            write_atomic(&marker, b"")
        } else {
            remove_if_exists(&marker)
        }
    }

    /// Leaves a command for the running engine.
    pub fn write_pending(&self, command: &PendingCommand) -> Result<()> {
        write_json(&self.file("command.pending"), command)
    }

    /// Reads and removes the pending command. The file is removed even when it is corrupt.
    pub fn take_pending(&self) -> Result<Option<PendingCommand>> {
        let path = self.file("command.pending");
        let Some(bytes) = read_optional(&path)? else {
            return Ok(None);
        };
        remove_if_exists(&path)?;
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|error| json_corrupt(&path, &error))
    }

    /// Publishes the engine's status.
    pub fn write_status(&self, status: &Status) -> Result<()> {
        write_json(&self.file("status.json"), status)
    }

    /// The last published status, if any.
    pub fn read_status(&self) -> Result<Option<Status>> {
        read_json(&self.file("status.json"))
    }

    /// Forgets the sync group but keeps the device key, device id and vault path.
    pub fn clear_group(&self) -> Result<()> {
        GROUP_FILES
            .iter()
            .try_for_each(|name| remove_if_exists(&self.file(name)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn lookup(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn fresh() -> (tempfile::TempDir, StateDir) {
        let tmp = tempfile::tempdir().unwrap();
        let state = StateDir::at(tmp.path().join("state")).unwrap();
        (tmp, state)
    }

    fn names(state: &StateDir) -> Vec<String> {
        let mut found: Vec<String> = std::fs::read_dir(state.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        found.sort();
        found
    }

    #[test]
    fn should_choose_base_dir_from_environment() {
        for windows in [true, false] {
            let env = lookup(&[
                ("NS_STATE_DIR", "/x"),
                ("LOCALAPPDATA", "C:\\L"),
                ("HOME", "/h"),
            ]);
            assert_eq!(base_dir_from(windows, &env).unwrap(), PathBuf::from("/x"));
        }
        let env = lookup(&[("LOCALAPPDATA", "C:\\L")]);
        let expected = PathBuf::from("C:\\L").join("neurospace");
        assert_eq!(base_dir_from(true, &env).unwrap(), expected);
        let env = lookup(&[("XDG_DATA_HOME", "/d"), ("HOME", "/h")]);
        let expected = PathBuf::from("/d").join("neurospace");
        assert_eq!(base_dir_from(false, &env).unwrap(), expected);
        let home_default = PathBuf::from("/h").join(".local/share").join("neurospace");
        for pairs in [
            &[("HOME", "/h")][..],
            &[("XDG_DATA_HOME", ""), ("HOME", "/h")][..],
        ] {
            assert_eq!(base_dir_from(false, &lookup(pairs)).unwrap(), home_default);
        }
        for windows in [true, false] {
            let result = base_dir_from(windows, &lookup(&[("NS_STATE_DIR", "")]));
            assert!(matches!(result, Err(SyncError::NoStateDir)));
        }
    }

    #[test]
    fn should_name_vault_dir_by_path_hash() {
        let name = vault_dir_name("C:\\V", true);
        assert_eq!(name.len(), 16);
        assert!(name.chars().all(|c| matches!(c, '0'..='9' | 'a'..='f')));
        assert_eq!(name, vault_dir_name("c:\\v", true));
        assert_ne!(vault_dir_name("/V", false), vault_dir_name("/v", false));
        assert_ne!(vault_dir_name("/a", false), vault_dir_name("/b", false));
    }

    #[test]
    fn should_create_state_dir_and_record_vault_path() {
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let base = tmp.path().join("base");
        let state = StateDir::for_vault_in(&base, &vault).unwrap();
        let canonical = std::fs::canonicalize(&vault).unwrap();
        let canonical_text = canonical.to_string_lossy().into_owned();
        let expected = base
            .join("vaults")
            .join(vault_dir_name(&canonical_text, cfg!(windows)));
        assert_eq!(state.path(), expected);
        let recorded = std::fs::read_to_string(state.file("vault-path.txt")).unwrap();
        assert_eq!(recorded, canonical_text);
        let again = StateDir::for_vault_in(&base, &vault).unwrap();
        assert_eq!(again.path(), state.path());
    }

    #[test]
    fn should_hold_one_engine_lock_at_a_time() {
        let (_tmp, state) = fresh();
        let first = state.lock().unwrap();
        assert!(matches!(state.lock(), Err(SyncError::AlreadyRunning)));
        drop(first);
        assert!(state.lock().is_ok());
    }

    #[test]
    fn should_create_secret_once() {
        let (_tmp, state) = fresh();
        assert_eq!(state.secret().unwrap(), None);
        let secret = state.ensure_secret().unwrap();
        assert_eq!(state.ensure_secret().unwrap(), secret);
        assert_eq!(state.secret().unwrap(), Some(secret));
        let text = std::fs::read_to_string(state.file("secret")).unwrap();
        assert_eq!(text, to_hex(&secret));
        assert_eq!(text.len(), 64);
        std::fs::write(state.file("secret"), format!("{text}\n")).unwrap();
        assert_eq!(state.secret().unwrap(), Some(secret));
        std::fs::write(state.file("secret"), "zz").unwrap();
        assert!(matches!(
            state.secret(),
            Err(SyncError::CorruptState { .. })
        ));
    }

    #[test]
    fn should_store_device_key() {
        let (_tmp, state) = fresh();
        assert_eq!(state.device_key().unwrap(), None);
        state.save_device_key(&[7; 32]).unwrap();
        assert_eq!(state.device_key().unwrap(), Some([7; 32]));
        assert_eq!(state.device_id().unwrap(), None);
        state.save_device_id(&[8; 32]).unwrap();
        assert_eq!(state.device_id().unwrap(), Some([8; 32]));
    }

    #[cfg(unix)]
    #[test]
    fn should_keep_state_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let vault = tmp.path().join("vault");
        std::fs::create_dir(&vault).unwrap();
        let base = tmp.path().join("base");
        let state = StateDir::for_vault_in(&base, &vault).unwrap();
        state.save_secret(&[1; 32]).unwrap();
        state.save_device_key(&[2; 32]).unwrap();
        state.save_peers(&[]).unwrap();
        drop(state.lock().unwrap());
        let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&base), 0o700);
        assert_eq!(mode(state.path()), 0o700);
        for name in [
            "secret",
            "device.key",
            "peers.json",
            "vault-path.txt",
            "engine.lock",
        ] {
            assert_eq!(mode(&state.file(name)), 0o600, "{name}");
        }
    }

    #[test]
    fn should_leave_no_temporary_file_when_the_rename_fails() {
        let (_tmp, state) = fresh();
        std::fs::create_dir(state.file("peers.json")).unwrap();
        assert!(matches!(state.save_peers(&[]), Err(SyncError::Io { .. })));
        assert!(names(&state).iter().all(|n| !n.ends_with(".tmp")));
    }

    #[test]
    fn should_not_quote_file_content_in_corrupt_state_errors() {
        let (_tmp, state) = fresh();
        for content in [
            r#"{"join": 5 "SECRETSECRET""#,
            r#"{"leave": "SECRETSECRET"}"#,
        ] {
            std::fs::write(state.file("command.pending"), content).unwrap();
            let error = state.take_pending().unwrap_err();
            assert!(matches!(error, SyncError::CorruptState { .. }));
            assert!(!error.to_string().contains("SECRETSECRET"), "{error}");
        }
        std::fs::write(state.file("config.json"), r#"{"mdns": "SECRETSECRET"}"#).unwrap();
        assert!(
            !state
                .config()
                .unwrap_err()
                .to_string()
                .contains("SECRETSECRET")
        );
    }

    #[test]
    fn should_redact_join_ticket_in_debug_output() {
        let join = PendingCommand::Join("ns1/SECRETSECRET".to_string());
        assert_eq!(format!("{join:?}"), r#"Join("<redacted>")"#);
        assert_eq!(format!("{:?}", PendingCommand::Leave(true)), "Leave(true)");
    }

    #[test]
    fn should_merge_peers_by_id() {
        let (_tmp, state) = fresh();
        assert!(state.peers().unwrap().is_empty());
        let (id_a, id_b) = ("aa".repeat(32), "bb".repeat(32));
        let peer = |id: &str, addrs: &[&str]| PeerEntry {
            id: id.to_string(),
            addrs: addrs.iter().map(|a| (*a).to_string()).collect(),
        };
        assert!(state.add_peer(peer(&id_a, &["1.2.3.4:5"])).unwrap());
        assert!(
            !state
                .add_peer(peer(&id_a, &["1.2.3.4:5", "6.7.8.9:1"]))
                .unwrap()
        );
        assert_eq!(
            state.peers().unwrap(),
            vec![peer(&id_a, &["1.2.3.4:5", "6.7.8.9:1"])]
        );
        assert!(state.add_peer(peer(&id_b, &[])).unwrap());
        assert_eq!(
            state.peers().unwrap(),
            vec![peer(&id_a, &["1.2.3.4:5", "6.7.8.9:1"]), peer(&id_b, &[])]
        );
    }

    #[test]
    fn should_keep_the_oldest_peers_when_the_list_is_full() {
        let (_tmp, state) = fresh();
        let id = |n: usize| format!("{n:064x}");
        let peer = |n: usize| PeerEntry {
            id: id(n),
            addrs: Vec::new(),
        };
        let full: Vec<PeerEntry> = (0..MAX_STORED_PEERS).map(peer).collect();
        state.save_peers(&full).unwrap();
        assert!(!state.add_peer(peer(MAX_STORED_PEERS)).unwrap());
        let stored = state.peers().unwrap();
        assert_eq!(stored.len(), MAX_STORED_PEERS);
        assert!(stored.iter().all(|entry| entry.id != id(MAX_STORED_PEERS)));
        let extra = PeerEntry {
            id: id(0),
            addrs: vec!["1.2.3.4:5".to_string()],
        };
        assert!(!state.add_peer(extra).unwrap());
        assert_eq!(state.peers().unwrap()[0].addrs, ["1.2.3.4:5"]);
    }

    #[cfg(unix)]
    #[test]
    fn should_not_keep_the_mode_of_a_stale_temporary_file() {
        use std::os::unix::fs::PermissionsExt;
        let (_tmp, state) = fresh();
        let tmp = state.file("secret.tmp");
        std::fs::write(&tmp, "old").unwrap();
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
        state.save_secret(&[1; 32]).unwrap();
        let mode = std::fs::metadata(state.file("secret"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn should_default_config_and_accept_partial_files() {
        let (_tmp, state) = fresh();
        let default = NetConfig {
            mode: NetMode::Internet,
            bind_port: None,
            mdns: true,
        };
        assert_eq!(state.config().unwrap(), default);
        let config_from = |json: &str| {
            std::fs::write(state.file("config.json"), json).unwrap();
            state.config()
        };
        let lan = NetConfig {
            mode: NetMode::Lan,
            bind_port: Some(4000),
            mdns: true,
        };
        assert_eq!(
            config_from(r#"{"mode":"lan","bind_port":4000}"#).unwrap(),
            lan
        );
        let quiet = NetConfig {
            mdns: false,
            ..default
        };
        assert_eq!(config_from(r#"{"mdns":false}"#).unwrap(), quiet);
        assert!(matches!(
            config_from("{nope"),
            Err(SyncError::CorruptState { .. })
        ));
    }

    #[test]
    fn should_round_trip_config_when_saved() {
        let (_tmp, state) = fresh();
        let lan = NetConfig {
            mode: NetMode::Lan,
            bind_port: Some(4500),
            mdns: false,
        };
        state.save_config(&lan).unwrap();
        assert_eq!(state.config().unwrap(), lan);
    }

    #[test]
    fn should_set_and_clear_fresh_join_marker() {
        let (_tmp, state) = fresh();
        assert!(!state.fresh_join());
        state.set_fresh_join(true).unwrap();
        assert!(state.fresh_join());
        state.set_fresh_join(false).unwrap();
        assert!(!state.fresh_join());
        state.set_fresh_join(false).unwrap();
    }

    #[test]
    fn should_hand_over_pending_command_once() {
        let (_tmp, state) = fresh();
        state
            .write_pending(&PendingCommand::Join("ns1/x".to_string()))
            .unwrap();
        let text = std::fs::read_to_string(state.file("command.pending")).unwrap();
        assert_eq!(text, r#"{"join":"ns1/x"}"#);
        assert_eq!(
            state.take_pending().unwrap(),
            Some(PendingCommand::Join("ns1/x".to_string()))
        );
        assert!(!state.file("command.pending").exists());
        assert_eq!(state.take_pending().unwrap(), None);
        state.write_pending(&PendingCommand::Leave(true)).unwrap();
        let text = std::fs::read_to_string(state.file("command.pending")).unwrap();
        assert_eq!(text, r#"{"leave":true}"#);
    }

    #[test]
    fn should_remove_corrupt_pending_command_before_failing() {
        let (_tmp, state) = fresh();
        std::fs::write(state.file("command.pending"), "garbage").unwrap();
        assert!(matches!(
            state.take_pending(),
            Err(SyncError::CorruptState { .. })
        ));
        assert!(!state.file("command.pending").exists());
    }

    #[test]
    fn should_round_trip_status() {
        let (_tmp, state) = fresh();
        assert_eq!(state.read_status().unwrap(), None);
        let status = Status {
            pid: 42,
            device_id: "ab".repeat(32),
            addresses: vec!["1.2.3.4:5".to_string()],
            peers: vec![PeerStatus {
                id: "cd".repeat(32),
                connected: true,
            }],
            notes: 3,
            blocked: vec![BlockedNote {
                path: "a.md".to_string(),
                reason: "not UTF-8".to_string(),
            }],
            rejected_entries: 2,
            relay: Some("https://relay.example./".to_string()),
            last_error: Some("boom".to_string()),
            updated_unix: 1_700_000_000,
        };
        state.write_status(&status).unwrap();
        assert_eq!(state.read_status().unwrap(), Some(status));
    }

    #[test]
    fn should_read_a_status_written_before_the_relay_field_existed() {
        let (_tmp, state) = fresh();
        let old = r#"{"pid":1,"device_id":"","addresses":[],"peers":[],"notes":0,"blocked":[],"last_error":null,"updated_unix":0}"#;
        std::fs::write(state.file("status.json"), old).unwrap();
        assert_eq!(state.read_status().unwrap().unwrap().relay, None);
    }

    #[test]
    fn should_forget_group_but_keep_device_key() {
        let (_tmp, state) = fresh();
        let all = [
            "device.key",
            "device.id",
            "vault-path.txt",
            "secret",
            "peers.json",
            "doc.snap",
            "doc.log",
            "base.json",
            "base.version",
            "fresh-join",
            "command.pending",
            "status.json",
        ];
        for name in all {
            std::fs::write(state.file(name), "x").unwrap();
        }
        let lock = state.lock().unwrap();
        state.clear_group().unwrap();
        drop(lock);
        assert_eq!(
            names(&state),
            ["device.id", "device.key", "engine.lock", "vault-path.txt"]
        );
        state.clear_group().unwrap();
    }

    #[test]
    fn should_write_state_files_atomically() {
        let (_tmp, state) = fresh();
        state.save_peers(&[]).unwrap();
        state.save_secret(&[1; 32]).unwrap();
        assert!(names(&state).iter().all(|n| !n.ends_with(".tmp")));
    }

    #[test]
    fn should_parse_hex_of_exactly_64_characters() {
        let lower = "ab".repeat(32);
        assert_eq!(from_hex32(&lower), Some([0xab; 32]));
        assert_eq!(from_hex32(&lower.to_uppercase()), Some([0xab; 32]));
        assert_eq!(from_hex32(&lower[..62]), None);
        assert_eq!(from_hex32(&format!("{lower}00")), None);
        assert_eq!(from_hex32(&"é".repeat(32)), None);
        assert_eq!(to_hex(&[0x0f, 0xa0]), "0fa0");
    }

    #[test]
    fn should_generate_distinct_random_bytes() {
        let first: [u8; 32] = random_bytes().unwrap();
        let second: [u8; 32] = random_bytes().unwrap();
        assert_ne!(first, second);
    }
}
