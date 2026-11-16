#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::time::Duration;

use common::{Net, Node, assert_converged, node};
use sb_sync::{
    Base, FRESH_JOIN_WAIT, Message, PeerEntry, PeerStatus, PendingCommand, SessionId, StateDir,
    SyncError, Tick, Ticket, to_hex,
};

const OTHER_SECRET: [u8; 32] = [8; 32];
const GROUP_SECRET: [u8; 32] = [7; 32];

fn state(node: &Node) -> &StateDir {
    &node.replica.local().state
}

fn address() -> SocketAddr {
    "127.0.0.1:4000".parse().unwrap()
}

fn ticket(device: u8, secret: [u8; 32]) -> Ticket {
    Ticket {
        device_id: [device; 32],
        secret,
        addrs: vec![address()],
    }
}

fn state_file_exists(node: &Node, name: &str) -> bool {
    state(node).file(name).exists()
}

fn add_peer(node: &Node, device: u8) {
    let entry = PeerEntry {
        id: to_hex(&[device; 32]),
        addrs: vec![],
    };
    state(node).add_peer(entry).unwrap();
}

/// Nodes 0 and 1 in one group with a session between them; node 0 reconciled `a.md` and `b.md`.
fn connected_pair_with_notes() -> (Net, Vec<Node>, SessionId) {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    let session = net.connect(&mut nodes, 0, 1);
    nodes[0].write("a.md", "one\n");
    nodes[0].write("b.md", "two\n");
    net.reconcile(&mut nodes, 0);
    net.settle(&mut nodes);
    (net, nodes, session)
}

fn join_with(node: &mut Node, ticket: &Ticket) -> Tick {
    node.replica.join(ticket, Duration::ZERO).unwrap()
}

/// Node 0 is the group (secret 7). Node 1 has its own, different group (secret 8) and has
/// reconciled its notes already.
fn group_and_newcomer(group: &[(&str, &str)], newcomer: &[(&str, &str)]) -> (Net, Vec<Node>) {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    state(&nodes[1]).save_secret(&OTHER_SECRET).unwrap();
    for (path, content) in group {
        nodes[0].write(path, content);
    }
    for (path, content) in newcomer {
        nodes[1].write(path, content);
    }
    net.reconcile(&mut nodes, 0);
    net.reconcile(&mut nodes, 1);
    (net, nodes)
}

fn join_group(nodes: &mut [Node], now: Duration) {
    let invitation = nodes[0].replica.invite().unwrap();
    nodes[1].replica.join(&invitation, now).unwrap();
}

/// Every content the node had is still in some file: a conflict may move it to another path.
fn assert_none_lost(before: &BTreeMap<String, String>, node: &Node) {
    let after: Vec<String> = node.files().into_values().collect();
    for (path, content) in before {
        assert!(after.contains(content), "the content of {path} was lost");
    }
}

/// The two replicas of `group_and_newcomer(.., [a, b])` after the first contact: «a» in
/// agreement, «b» once as a note and once as a conflict copy, «c» from the group.
fn assert_contents_of_the_wait_scenario(nodes: &mut [Node]) {
    let files = nodes[1].files();
    assert_eq!(files.len(), 4, "{files:?}");
    assert_eq!(files["a.md"], "same\n");
    assert_eq!(files["c.md"], "group only\n");
    let mut b_versions: Vec<&str> = files
        .iter()
        .filter(|(path, _)| path.starts_with("b"))
        .map(|(_, content)| content.as_str())
        .collect();
    b_versions.sort_unstable();
    assert_eq!(b_versions, ["mine\n", "theirs\n"]);
    assert_converged(nodes);
}

const GROUP_NOTES: [(&str, &str); 3] = [
    ("a.md", "same\n"),
    ("b.md", "theirs\n"),
    ("c.md", "group only\n"),
];
const NEWCOMER_NOTES: [(&str, &str); 2] = [("a.md", "same\n"), ("b.md", "mine\n")];

#[test]
fn should_only_add_the_peer_when_the_ticket_is_for_the_same_group() {
    let (_net, mut nodes, _session) = connected_pair_with_notes();
    let local = nodes[0].replica.local();
    let (version, base) = (local.doc.version(), local.base.clone());
    let base_file = std::fs::read(state(&nodes[0]).file("base.json")).unwrap();
    let files = nodes[0].files();

    let tick = join_with(&mut nodes[0], &ticket(9, GROUP_SECRET));

    assert!(!tick.secret_changed);
    assert!(tick.close_sessions.is_empty());
    let peers = state(&nodes[0]).peers().unwrap();
    let expected = PeerEntry {
        id: to_hex(&[9; 32]),
        addrs: vec!["127.0.0.1:4000".to_string()],
    };
    assert!(peers.contains(&expected), "{peers:?}");
    let local = nodes[0].replica.local();
    assert_eq!(local.doc.version(), version);
    assert_eq!(local.doc.alive_notes().len(), 2);
    assert_eq!(local.base, base);
    let base_after = std::fs::read(state(&nodes[0]).file("base.json")).unwrap();
    assert_eq!(base_after, base_file);
    assert!(!state(&nodes[0]).fresh_join());
    assert!(!nodes[0].replica.is_waiting_for_first_state());
    assert_eq!(nodes[0].files(), files);
}

#[test]
fn should_replace_sync_state_when_joining_another_group() {
    let (_net, mut nodes, session) = connected_pair_with_notes();
    add_peer(&nodes[0], 2);
    let files = nodes[0].files();

    let tick = join_with(&mut nodes[0], &ticket(9, OTHER_SECRET));

    assert_eq!(tick.close_sessions, [session]);
    assert!(tick.secret_changed);
    assert!(nodes[0].replica.connected_peers().is_empty());
    assert_eq!(state(&nodes[0]).secret().unwrap(), Some(OTHER_SECRET));
    let only_new = vec![PeerEntry {
        id: to_hex(&[9; 32]),
        addrs: vec!["127.0.0.1:4000".to_string()],
    }];
    assert_eq!(state(&nodes[0]).peers().unwrap(), only_new);
    assert!(nodes[0].replica.local().doc.alive_notes().is_empty());
    assert!(nodes[0].replica.local().base.is_empty());
    assert!(Base::load(state(&nodes[0])).unwrap().is_empty());
    assert!(state(&nodes[0]).fresh_join());
    assert!(nodes[0].replica.is_waiting_for_first_state());
    assert_eq!(nodes[0].files(), files);
}

#[test]
fn should_wait_for_a_first_state_before_reconciling() {
    let (mut net, mut nodes) = group_and_newcomer(&GROUP_NOTES, &NEWCOMER_NOTES);
    let files = nodes[1].files();
    join_group(&mut nodes, Duration::ZERO);
    assert_eq!(nodes[1].files(), files);

    assert!(nodes[1].replica.reconcile().unwrap().is_empty());
    assert!(nodes[1].replica.local().doc.alive_notes().is_empty());

    net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);

    assert_contents_of_the_wait_scenario(&mut nodes);
    assert_eq!(nodes[0].files(), nodes[1].files());
    assert_eq!(nodes[1].replica.local().doc.alive_notes().len(), 4);
    assert!(!nodes[1].replica.is_waiting_for_first_state());
    assert!(!state(&nodes[1]).fresh_join());
    assert_none_lost(&files, &nodes[1]);
}

#[test]
fn should_stop_waiting_after_the_deadline() {
    let (mut net, mut nodes) = group_and_newcomer(&GROUP_NOTES, &NEWCOMER_NOTES);
    let files = nodes[1].files();
    join_group(&mut nodes, Duration::ZERO);

    net.tick(&mut nodes, 1, FRESH_JOIN_WAIT - Duration::from_secs(1));
    assert!(nodes[1].replica.is_waiting_for_first_state());
    assert!(nodes[1].replica.local().doc.alive_notes().is_empty());

    net.tick(&mut nodes, 1, FRESH_JOIN_WAIT);
    assert!(!nodes[1].replica.is_waiting_for_first_state());
    assert!(!state(&nodes[1]).fresh_join());
    assert_eq!(nodes[1].replica.local().doc.alive_notes().len(), 2);
    assert_eq!(nodes[1].files(), files);

    net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);
    assert_contents_of_the_wait_scenario(&mut nodes);
    assert_none_lost(&files, &nodes[1]);
}

/// A replica whose notes all match the group's adds nothing to the document while it waits,
/// so it sends the group no edits: no text is stored twice.
#[test]
fn should_not_store_a_notes_text_twice_when_the_first_state_arrives_in_time() {
    let notes = [("a.md", "same\n")];
    let (mut net, mut nodes) = group_and_newcomer(&notes, &notes);
    join_group(&mut nodes, Duration::ZERO);
    net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);

    let edits_sent_to_the_group = net.deliveries.iter().any(|(to, message)| {
        matches!(message, Message::Updates { updates, .. } if *to == 0 && !updates.is_empty())
    });
    assert!(!edits_sent_to_the_group);
    assert_eq!(nodes[1].replica.local().doc.alive_notes().len(), 1);
    assert_converged(&mut nodes);
}

#[test]
fn should_start_the_wait_at_the_first_tick_when_opened_with_the_marker_set() {
    let (mut net, mut nodes) = group_and_newcomer(&[], &NEWCOMER_NOTES);
    join_group(&mut nodes, Duration::ZERO);
    let reopened = reopen(nodes.remove(1));
    nodes.push(reopened);
    assert!(nodes[1].replica.is_waiting_for_first_state());

    let late = Duration::from_secs(1000);
    net.tick(&mut nodes, 1, late);
    assert!(nodes[1].replica.is_waiting_for_first_state());
    net.tick(&mut nodes, 1, late + FRESH_JOIN_WAIT);
    assert!(!nodes[1].replica.is_waiting_for_first_state());
}

fn reopen(node: Node) -> Node {
    let Node {
        replica,
        vault_dir,
        state_dir,
        id,
    } = node;
    drop(replica);
    let state = StateDir::at(state_dir.path()).unwrap();
    let vault = sb_core::Vault::open(vault_dir.path()).unwrap();
    let replica = sb_sync::Replica::open(vault, state, id).unwrap();
    Node {
        replica,
        vault_dir,
        state_dir,
        id,
    }
}

#[test]
fn should_end_the_wait_on_an_empty_answer() {
    let (mut net, mut nodes) = group_and_newcomer(&[], &NEWCOMER_NOTES);
    join_group(&mut nodes, Duration::ZERO);
    assert!(nodes[1].replica.is_waiting_for_first_state());

    net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);

    assert!(!nodes[1].replica.is_waiting_for_first_state());
    assert_eq!(nodes[0].files(), nodes[1].files());
    assert_eq!(nodes[0].files().len(), 2);
    assert_converged(&mut nodes);
}

#[test]
fn should_never_delete_local_notes_when_joining() {
    let (mut net, mut nodes) = group_and_newcomer(&[], &NEWCOMER_NOTES);
    let files = nodes[1].files();
    assert!(!nodes[1].replica.local().base.is_empty());
    join_group(&mut nodes, Duration::ZERO);
    assert_none_lost(&files, &nodes[1]);

    net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);

    assert_eq!(nodes[1].files(), files);
    assert!(nodes[1].trash().is_empty(), "{:?}", nodes[1].trash());
}

#[test]
fn should_turn_sync_off_when_leaving() {
    let (_net, mut nodes, session) = connected_pair_with_notes();
    state(&nodes[0]).save_device_key(&[3; 32]).unwrap();
    add_peer(&nodes[0], 2);
    state(&nodes[0]).set_fresh_join(true).unwrap();
    let files = nodes[0].files();

    let tick = nodes[0].replica.leave().unwrap();

    assert!(tick.left);
    assert_eq!(tick.close_sessions, [session]);
    let gone = ["secret", "peers.json", "doc.snap", "doc.log", "base.json"];
    for name in gone.into_iter().chain(["fresh-join"]) {
        assert!(!state_file_exists(&nodes[0], name), "{name} is still there");
    }
    assert!(state_file_exists(&nodes[0], "device.key"));
    assert!(!nodes[0].replica.is_waiting_for_first_state());
    assert_eq!(nodes[0].files(), files);
}

#[test]
fn should_carry_out_a_pending_command_on_tick() {
    let mut nodes = vec![node(1)];
    let mut net = Net::new();
    let invitation = ticket(9, OTHER_SECRET);
    state(&nodes[0])
        .write_pending(&PendingCommand::Join(invitation.to_string()))
        .unwrap();
    let tick = net.tick(&mut nodes, 0, Duration::ZERO);
    assert!(tick.secret_changed);
    assert_eq!(state(&nodes[0]).secret().unwrap(), Some(OTHER_SECRET));
    assert!(!state_file_exists(&nodes[0], "command.pending"));

    state(&nodes[0])
        .write_pending(&PendingCommand::Leave(true))
        .unwrap();
    let tick = net.tick(&mut nodes, 0, Duration::from_secs(5));
    assert!(tick.left);
    assert!(!state_file_exists(&nodes[0], "command.pending"));
    assert_eq!(state(&nodes[0]).secret().unwrap(), None);
}

#[test]
fn should_drop_a_bad_pending_command_and_report_it() {
    let own_ticket = PendingCommand::Join(ticket(1, OTHER_SECRET).to_string());
    let own_ticket = serde_json::to_string(&own_ticket).unwrap();
    for content in [
        r#"{"join":"not-a-ticket-garbage"}"#.to_string(),
        "{ garbage".to_string(),
        own_ticket,
    ] {
        // A fresh node per case: a report left over from an earlier case must not satisfy this one.
        let mut nodes = vec![node(1)];
        let mut net = Net::new();
        std::fs::write(state(&nodes[0]).file("command.pending"), content).unwrap();
        let tick = net.tick(&mut nodes, 0, Duration::ZERO);
        assert!(!tick.secret_changed && !tick.left);
        assert!(!state_file_exists(&nodes[0], "command.pending"));
        assert_eq!(state(&nodes[0]).secret().unwrap(), Some(GROUP_SECRET));
        let status = state(&nodes[0]).read_status().unwrap().unwrap();
        let reported = status.last_error.expect("the problem is reported");
        assert!(!reported.contains("garbage"), "{reported}");
    }
}

#[test]
fn should_clear_the_reported_error_when_a_later_command_succeeds() {
    let mut nodes = vec![node(1)];
    let mut net = Net::new();
    std::fs::write(state(&nodes[0]).file("command.pending"), "{ garbage").unwrap();
    net.tick(&mut nodes, 0, Duration::ZERO);
    assert!(nodes[0].replica.status().last_error.is_some());

    let command = PendingCommand::Join(ticket(9, OTHER_SECRET).to_string());
    state(&nodes[0]).write_pending(&command).unwrap();
    net.tick(&mut nodes, 0, Duration::from_secs(5));

    assert_eq!(nodes[0].replica.status().last_error, None);
    let status = state(&nodes[0]).read_status().unwrap().unwrap();
    assert_eq!(status.last_error, None);
}

#[test]
fn should_keep_the_tick_of_a_carried_out_command_when_a_later_step_fails() {
    let mut node = node(1);
    let command = PendingCommand::Join(ticket(9, OTHER_SECRET).to_string());
    state(&node).write_pending(&command).unwrap();
    // A directory in the place of status.json makes every status write fail.
    std::fs::create_dir(state(&node).file("status.json")).unwrap();

    let tick = node.replica.tick(Duration::ZERO).unwrap();

    assert!(tick.secret_changed);
    assert_eq!(state(&node).secret().unwrap(), Some(OTHER_SECRET));
    assert!(node.replica.status().last_error.is_some());
}

#[test]
fn should_still_fail_the_tick_when_no_command_ran_and_the_status_cannot_be_written() {
    let mut node = node(1);
    std::fs::create_dir(state(&node).file("status.json")).unwrap();
    assert!(node.replica.tick(Duration::ZERO).is_err());
}

#[test]
fn should_refuse_a_ticket_for_this_device() {
    let (_net, mut nodes, _session) = connected_pair_with_notes();
    let files = nodes[0].files();
    let peers = state(&nodes[0]).peers().unwrap();
    for secret in [GROUP_SECRET, OTHER_SECRET] {
        let own = ticket(1, secret);
        let result = nodes[0].replica.join(&own, Duration::ZERO);
        assert!(matches!(result, Err(SyncError::InvalidTicket(_))));
    }
    assert_eq!(state(&nodes[0]).secret().unwrap(), Some(GROUP_SECRET));
    assert_eq!(state(&nodes[0]).peers().unwrap(), peers);
    assert!(!state(&nodes[0]).fresh_join());
    assert_eq!(nodes[0].replica.local().doc.alive_notes().len(), 2);
    assert_eq!(nodes[0].replica.connected_peers(), [[2; 32]]);
    assert_eq!(nodes[0].files(), files);
}

#[test]
fn should_pick_up_peers_added_while_running() {
    let mut nodes = [node(1)];
    add_peer(&nodes[0], 9);
    let opened = nodes[0].replica.session_opened(5, [2; 32]);
    let peers = opened.iter().find_map(|outgoing| match &outgoing.message {
        Message::Peers(ids) => Some(ids.clone()),
        _ => None,
    });
    assert_eq!(peers.unwrap(), vec![[9; 32]]);
}

#[test]
fn should_write_status_on_tick() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    net.connect(&mut nodes, 0, 1);
    nodes[0].replica.set_addresses(vec![address()]);
    add_peer(&nodes[0], 2);
    add_peer(&nodes[0], 5);
    nodes[0].write("a.md", "one\n");
    nodes[0].write("b.md", "two\n");
    std::fs::write(nodes[0].vault_dir.path().join("bad.md"), [0xff, 0xfe]).unwrap();
    net.reconcile(&mut nodes, 0);

    net.tick(&mut nodes, 0, Duration::from_secs(5));

    let status = state(&nodes[0]).read_status().unwrap().unwrap();
    assert_eq!(status.pid, std::process::id());
    assert_eq!(status.device_id, to_hex(&[1; 32]));
    assert_eq!(status.addresses, ["127.0.0.1:4000"]);
    let peers = [(&[2u8; 32], true), (&[5; 32], false)].map(|(id, connected)| PeerStatus {
        id: to_hex(id),
        connected,
    });
    assert_eq!(status.peers, peers);
    assert_eq!(status.notes, 2);
    assert_eq!(status.blocked.len(), 1);
    assert_eq!(status.blocked[0].path, "bad.md");
    assert!(status.updated_unix > 0);
}

#[test]
fn should_make_a_ticket_with_the_group_secret() {
    let mut nodes = [node(1)];
    nodes[0].replica.set_addresses(vec![address()]);
    let invitation = nodes[0].replica.invite().unwrap();
    assert_eq!(
        invitation,
        Ticket {
            device_id: [1; 32],
            secret: GROUP_SECRET,
            addrs: vec![address()]
        }
    );
}
