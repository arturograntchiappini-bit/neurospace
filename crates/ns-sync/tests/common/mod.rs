#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    missing_docs,
    dead_code
)]

use std::collections::{BTreeMap, VecDeque};
use std::fs;
use std::path::Path;
use std::time::Duration;

use ns_core::Vault;
use ns_sync::{DeviceId, Message, Outgoing, Replica, SessionId, StateDir, Tick};

const DELIVERY_LIMIT: usize = 10_000;

/// A message in flight: the node that receives it, and what it carries.
pub type Delivery = (usize, Outgoing);

pub struct Node {
    pub replica: Replica,
    pub vault_dir: tempfile::TempDir,
    pub state_dir: tempfile::TempDir,
    pub id: DeviceId,
}

/// A node with an empty vault, device id `[n; 32]` and a group secret already stored.
pub fn node(n: u8) -> Node {
    let vault_dir = tempfile::tempdir().unwrap();
    let state_dir = tempfile::tempdir().unwrap();
    let state = StateDir::at(state_dir.path()).unwrap();
    state.save_secret(&[7; 32]).unwrap();
    let vault = Vault::open(vault_dir.path()).unwrap();
    let id = [n; 32];
    let replica = Replica::open(vault, state, id).unwrap();
    Node {
        replica,
        vault_dir,
        state_dir,
        id,
    }
}

impl Node {
    pub fn write(&self, path: &str, content: &str) {
        let full = self.vault_dir.path().join(path);
        fs::create_dir_all(full.parent().unwrap()).unwrap();
        fs::write(full, content).unwrap();
    }

    pub fn remove(&self, path: &str) {
        fs::remove_file(self.vault_dir.path().join(path)).unwrap();
    }

    pub fn rename(&self, from: &str, to: &str) {
        let target = self.vault_dir.path().join(to);
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::rename(self.vault_dir.path().join(from), target).unwrap();
    }

    /// Every `.md` outside dot-directories: relative path with `/` to content.
    pub fn files(&self) -> BTreeMap<String, String> {
        let mut found = BTreeMap::new();
        collect_notes(self.vault_dir.path(), "", &mut found);
        found
    }

    /// The relative paths of everything in the vault's trash.
    pub fn trash(&self) -> Vec<String> {
        let mut found = Vec::new();
        collect_all(&self.replica.local().vault.trash_dir(), "", &mut found);
        found
    }
}

fn collect_notes(dir: &Path, prefix: &str, found: &mut BTreeMap<String, String>) {
    for entry in fs::read_dir(dir).unwrap() {
        let entry = entry.unwrap();
        let name = entry.file_name().into_string().unwrap();
        if name.starts_with('.') {
            continue;
        }
        let relative = format!("{prefix}{name}");
        if entry.file_type().unwrap().is_dir() {
            collect_notes(&entry.path(), &format!("{relative}/"), found);
        } else if name.ends_with(".md") {
            found.insert(relative, fs::read_to_string(entry.path()).unwrap());
        }
    }
}

fn collect_all(dir: &Path, prefix: &str, found: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let entry = entry.unwrap();
        let relative = format!("{prefix}{}", entry.file_name().into_string().unwrap());
        if entry.file_type().unwrap().is_dir() {
            collect_all(&entry.path(), &format!("{relative}/"), found);
        } else {
            found.push(relative);
        }
    }
}

/// An in-memory network between nodes: links and the queue of messages in flight.
pub struct Net {
    next_session: SessionId,
    links: Vec<(SessionId, usize, usize)>,
    queue: VecDeque<Delivery>,
    /// Every message delivered so far, with the node that received it.
    pub deliveries: Vec<(usize, Message)>,
}

impl Net {
    pub fn new() -> Net {
        Net {
            next_session: 1,
            links: Vec::new(),
            queue: VecDeque::new(),
            deliveries: Vec::new(),
        }
    }

    /// Opens a session between two nodes and queues what each side says first.
    pub fn connect(&mut self, nodes: &mut [Node], a: usize, b: usize) -> SessionId {
        let session = self.next_session;
        self.next_session += 1;
        self.links.push((session, a, b));
        let from_a = nodes[a].replica.session_opened(session, nodes[b].id);
        let from_b = nodes[b].replica.session_opened(session, nodes[a].id);
        self.enqueue(a, from_a);
        self.enqueue(b, from_b);
        session
    }

    /// Closes every session between two nodes and drops what was still in flight on them.
    pub fn disconnect(&mut self, nodes: &mut [Node], a: usize, b: usize) {
        let closing: Vec<SessionId> = self
            .links
            .iter()
            .filter(|&&(_, x, y)| (x, y) == (a, b) || (x, y) == (b, a))
            .map(|&(session, _, _)| session)
            .collect();
        for session in &closing {
            nodes[a].replica.session_closed(*session);
            nodes[b].replica.session_closed(*session);
        }
        self.links
            .retain(|(session, _, _)| !closing.contains(session));
        self.queue
            .retain(|(_, outgoing)| !closing.contains(&outgoing.session));
    }

    /// One local round on node `i`; what it sends is queued.
    pub fn reconcile(&mut self, nodes: &mut [Node], i: usize) {
        let outgoing = nodes[i].replica.reconcile().unwrap();
        self.enqueue(i, outgoing);
    }

    /// One tick of node `i` at engine time `now`: the sessions it closes are closed on both
    /// ends, what it sends is queued, and the tick is returned.
    pub fn tick(&mut self, nodes: &mut [Node], i: usize, now: Duration) -> Tick {
        let tick = nodes[i].replica.tick(now).unwrap();
        for session in &tick.close_sessions {
            self.close(nodes, *session);
        }
        self.enqueue(i, tick.outgoing.clone());
        tick
    }

    fn close(&mut self, nodes: &mut [Node], session: SessionId) {
        let Some(position) = self.links.iter().position(|link| link.0 == session) else {
            return;
        };
        let (_, a, b) = self.links.remove(position);
        nodes[a].replica.session_closed(session);
        nodes[b].replica.session_closed(session);
        self.queue
            .retain(|(_, outgoing)| outgoing.session != session);
    }

    /// Delivers queued messages, and the replies they cause, until none are left.
    pub fn settle(&mut self, nodes: &mut [Node]) {
        self.settle_with(nodes, |batch| batch);
    }

    /// Like `settle`, but each batch of queued messages passes through `reorder` before delivery.
    pub fn settle_with(
        &mut self,
        nodes: &mut [Node],
        reorder: impl Fn(Vec<Delivery>) -> Vec<Delivery>,
    ) {
        let mut delivered = 0;
        while !self.queue.is_empty() {
            let batch = reorder(self.queue.drain(..).collect());
            for (to, outgoing) in batch {
                delivered += 1;
                assert!(
                    delivered <= DELIVERY_LIMIT,
                    "more than {DELIVERY_LIMIT} deliveries: the replicas keep answering each other"
                );
                self.deliveries.push((to, outgoing.message.clone()));
                let replies = nodes[to]
                    .replica
                    .on_message(outgoing.session, outgoing.message)
                    .unwrap();
                self.enqueue(to, replies);
            }
        }
    }

    fn enqueue(&mut self, from: usize, outgoing: Vec<Outgoing>) {
        for message in outgoing {
            let Some(&(_, a, b)) = self.links.iter().find(|link| link.0 == message.session) else {
                continue;
            };
            self.queue
                .push_back((if from == a { b } else { a }, message));
        }
    }
}

/// All nodes hold the same files, and one more round on each changes nothing and sends nothing.
pub fn assert_converged(nodes: &mut [Node]) {
    let first = nodes[0].files();
    for (index, node) in nodes.iter().enumerate() {
        assert_eq!(node.files(), first, "node {index} differs from node 0");
    }
    for (index, node) in nodes.iter_mut().enumerate() {
        let outgoing = node.replica.reconcile().unwrap();
        assert!(
            outgoing.is_empty(),
            "node {index} still had something to send"
        );
        assert_eq!(
            node.files(),
            first,
            "node {index} changed files in a further round"
        );
    }
}
