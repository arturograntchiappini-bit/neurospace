#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, missing_docs)]

mod common;

use common::{Net, Node, assert_converged, node};
use sb_sync::{MAX_PEERS_PER_MESSAGE, Message, SyncError, from_hex32};

/// Reconciles each node in index order, then settles; twice.
fn sync(net: &mut Net, nodes: &mut [Node]) {
    for _ in 0..2 {
        for i in 0..nodes.len() {
            net.reconcile(nodes, i);
        }
        net.settle(nodes);
    }
}

/// Two connected nodes where A's `path` already reached B.
fn shared_pair(path: &str, content: &str) -> (Net, Vec<Node>) {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    net.connect(&mut nodes, 0, 1);
    nodes[0].write(path, content);
    sync(&mut net, &mut nodes);
    assert_eq!(
        nodes[1].files().get(path).map(String::as_str),
        Some(content)
    );
    (net, nodes)
}

fn is_conflict_name(name: &str, stem: &str) -> bool {
    name.strip_prefix(stem)
        .and_then(|rest| rest.strip_prefix(" (conflict "))
        .and_then(|rest| rest.strip_suffix(").md"))
        .is_some_and(|id| id.len() == 8 && id.chars().all(|c| c.is_ascii_hexdigit()))
}

#[test]
fn should_copy_notes_to_an_empty_replica() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    net.connect(&mut nodes, 0, 1);
    nodes[0].write("a.md", "alpha\n");
    nodes[0].write("d/b.md", "beta\n");
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[1].files(), nodes[0].files());
    assert_eq!(nodes[1].files().len(), 2);
    assert_converged(&mut nodes);
}

#[test]
fn should_merge_edits_of_different_lines() {
    let (mut net, mut nodes) = shared_pair("n.md", "l1\nl2\nl3\n");
    net.disconnect(&mut nodes, 0, 1);
    nodes[0].write("n.md", "l1 A\nl2\nl3\n");
    nodes[1].write("n.md", "l1\nl2\nl3 B\n");
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    for node in &nodes {
        assert_eq!(node.files()["n.md"], "l1 A\nl2\nl3 B\n");
    }
    assert_converged(&mut nodes);
}

#[test]
fn should_keep_both_versions_of_a_line_edited_on_two_replicas() {
    let (mut net, mut nodes) = shared_pair("n.md", "l1\nl2\nl3\n");
    net.disconnect(&mut nodes, 0, 1);
    nodes[0].write("n.md", "l1\nl2 A\nl3\n");
    nodes[1].write("n.md", "l1\nl2 B\nl3\n");
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[0].files().len(), 1);
    let text = &nodes[0].files()["n.md"];
    assert!(text.contains("l2 A") && text.contains("l2 B"), "{text:?}");
    assert_eq!(text.lines().count(), 4, "{text:?}");
    assert_converged(&mut nodes);
}

#[test]
fn should_propagate_a_delete_into_the_trash() {
    let (mut net, mut nodes) = shared_pair("n.md", "text\n");
    nodes[0].remove("n.md");
    sync(&mut net, &mut nodes);
    assert!(!nodes[1].files().contains_key("n.md"));
    assert_eq!(nodes[1].trash().len(), 1);
    assert!(nodes[0].trash().is_empty());
    assert_converged(&mut nodes);
}

#[test]
fn should_keep_a_note_edited_while_it_was_deleted_elsewhere() {
    let (mut net, mut nodes) = shared_pair("n.md", "text\n");
    net.disconnect(&mut nodes, 0, 1);
    nodes[0].remove("n.md");
    nodes[1].write("n.md", "edited\n");
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    for node in &nodes {
        assert_eq!(node.files()["n.md"], "edited\n");
    }
    assert_converged(&mut nodes);
}

#[test]
fn should_delete_a_note_removed_on_both_replicas_at_different_states() {
    let (mut net, mut nodes) = shared_pair("n.md", "v0\n");
    net.disconnect(&mut nodes, 0, 1);
    nodes[0].remove("n.md");
    nodes[1].write("n.md", "v1\n");
    net.reconcile(&mut nodes, 1);
    nodes[1].remove("n.md");
    net.reconcile(&mut nodes, 1);
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    assert!(nodes[0].files().is_empty() && nodes[1].files().is_empty());
    assert_converged(&mut nodes);
}

#[test]
fn should_propagate_a_rename_as_a_rename() {
    let (mut net, mut nodes) = shared_pair("old.md", "text\n");
    nodes[0].rename("old.md", "sub/new.md");
    sync(&mut net, &mut nodes);
    let files = nodes[1].files();
    assert_eq!(files.keys().collect::<Vec<_>>(), ["sub/new.md"]);
    assert_eq!(files["sub/new.md"], "text\n");
    assert!(nodes[1].trash().is_empty());
    assert_converged(&mut nodes);
}

#[test]
fn should_carry_an_edit_to_the_renamed_note() {
    let (mut net, mut nodes) = shared_pair("old.md", "l1\nl2\n");
    net.disconnect(&mut nodes, 0, 1);
    nodes[0].rename("old.md", "new.md");
    nodes[1].write("old.md", "l1\nl2 B\n");
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    for node in &nodes {
        assert_eq!(node.files().keys().collect::<Vec<_>>(), ["new.md"]);
        assert_eq!(node.files()["new.md"], "l1\nl2 B\n");
    }
    assert_converged(&mut nodes);
}

#[test]
fn should_end_with_one_note_after_two_concurrent_renames() {
    let (mut net, mut nodes) = shared_pair("n.md", "text\n");
    net.disconnect(&mut nodes, 0, 1);
    nodes[0].rename("n.md", "a.md");
    nodes[1].rename("n.md", "b.md");
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[0].files().len(), 1);
    assert_eq!(nodes[0].files().values().next().unwrap(), "text\n");
    assert_converged(&mut nodes);
}

#[test]
fn should_dedupe_a_path_created_twice_with_equal_content() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    for i in 0..2 {
        nodes[i].write("daily.md", "same\n");
        net.reconcile(&mut nodes, i);
    }
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    for node in &nodes {
        assert_eq!(node.files().keys().collect::<Vec<_>>(), ["daily.md"]);
        assert_eq!(node.files()["daily.md"], "same\n");
    }
    assert_converged(&mut nodes);
}

#[test]
fn should_keep_both_when_a_path_is_created_twice_with_different_content() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    nodes[0].write("daily.md", "A\n");
    nodes[1].write("daily.md", "B\n");
    for i in 0..2 {
        net.reconcile(&mut nodes, i);
    }
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    let files = nodes[0].files();
    assert_eq!(files.len(), 2, "{files:?}");
    let copy = files
        .keys()
        .find(|name| is_conflict_name(name, "daily"))
        .unwrap_or_else(|| panic!("no conflict copy in {files:?}"));
    assert!(files.contains_key("daily.md"));
    let mut texts = [files["daily.md"].as_str(), files[copy].as_str()];
    texts.sort_unstable();
    assert_eq!(texts, ["A\n", "B\n"]);
    assert_converged(&mut nodes);
}

#[test]
fn should_not_delete_remotely_when_a_file_becomes_unreadable() {
    let (mut net, mut nodes) = shared_pair("n.md", "old text\n");
    std::fs::write(nodes[0].vault_dir.path().join("n.md"), [0xff, 0xfe]).unwrap();
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[1].files()["n.md"], "old text\n");
    let on_disk = std::fs::read(nodes[0].vault_dir.path().join("n.md")).unwrap();
    assert_eq!(on_disk, [0xff, 0xfe]);
    let blocked = &nodes[0].replica.last_outcome().blocked;
    assert!(
        blocked.iter().any(|note| note.path == "n.md"),
        "{blocked:?}"
    );
}

#[test]
fn should_converge_through_a_middle_replica() {
    let mut nodes = vec![node(1), node(2), node(3)];
    let mut net = Net::new();
    net.connect(&mut nodes, 0, 1);
    net.connect(&mut nodes, 1, 2);
    nodes[0].write("a.md", "from a\n");
    nodes[2].write("c.md", "from c\n");
    sync(&mut net, &mut nodes);
    sync(&mut net, &mut nodes);
    for node in &nodes {
        assert_eq!(node.files().len(), 2);
    }
    assert_converged(&mut nodes);
}

/// 30 changes on node 0, each followed by a local round: creates, edits, renames, deletes.
fn make_thirty_changes(net: &mut Net, nodes: &mut [Node]) {
    for i in 0..30 {
        match i % 6 {
            0 => nodes[0].write(&format!("f{i}.md"), &format!("created {i}\n")),
            1 => nodes[0].write(&format!("f{}.md", i - 1), &format!("edited {i}\n")),
            2 => nodes[0].rename(&format!("f{}.md", i - 2), &format!("d/f{}.md", i - 2)),
            3 => nodes[0].write(&format!("g{i}.md"), &format!("scratch {i}\n")),
            4 => nodes[0].remove(&format!("g{}.md", i - 1)),
            _ => nodes[0].write("h.md", &format!("h {i}\n")),
        }
        net.reconcile(nodes, 0);
    }
}

#[test]
fn should_catch_up_after_being_offline() {
    let (mut net, mut nodes) = shared_pair("seed.md", "seed\n");
    net.disconnect(&mut nodes, 0, 1);
    make_thirty_changes(&mut net, &mut nodes);
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[1].files(), nodes[0].files());
    assert_converged(&mut nodes);
}

#[test]
fn should_tolerate_duplicate_and_reordered_updates() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    net.connect(&mut nodes, 0, 1);
    nodes[0].write("a.md", "one\n");
    nodes[0].write("d/b.md", "two\n");
    nodes[1].write("c.md", "three\n");
    for i in 0..2 {
        net.reconcile(&mut nodes, i);
    }
    let hostile = |batch: Vec<common::Delivery>| {
        let mut doubled: Vec<_> = batch
            .into_iter()
            .flat_map(|item| {
                let twice = matches!(item.1.message, Message::Updates { .. });
                let copies = if twice { 2 } else { 1 };
                std::iter::repeat_n(item, copies)
            })
            .collect();
        doubled.reverse();
        doubled
    };
    net.settle_with(&mut nodes, hostile);
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[0].files().len(), 3);
    assert_converged(&mut nodes);
}

#[test]
fn should_ask_again_when_updates_arrive_without_their_history() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    let session = net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);
    nodes[0].write("a.md", "first\n");
    nodes[0].replica.reconcile().unwrap();
    let after_first = nodes[0].replica.local().doc.version();
    nodes[0].write("a.md", "first\nsecond\n");
    nodes[0].replica.reconcile().unwrap();
    let doc = &nodes[0].replica.local().doc;
    let message = Message::Updates {
        version: doc.version(),
        updates: doc.updates_since(&after_first).unwrap(),
    };
    let replies = nodes[1].replica.on_message(session, message).unwrap();
    assert!(
        replies
            .iter()
            .any(|reply| reply.session == session && matches!(reply.message, Message::Version(_))),
        "{replies:?}"
    );
}

#[test]
fn should_not_echo_updates_back_to_their_sender() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);
    net.deliveries.clear();
    nodes[0].write("a.md", "news\n");
    net.reconcile(&mut nodes, 0);
    net.settle(&mut nodes);
    assert_eq!(nodes[1].files()["a.md"], "news\n");
    let echoed = net.deliveries.iter().any(|(to, message)| {
        *to == 0 && matches!(message, Message::Updates { updates, .. } if !updates.is_empty())
    });
    assert!(!echoed, "B sent data back to A");
}

#[test]
fn should_report_connected_peers() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    assert!(nodes[0].replica.connected_peers().is_empty());
    net.connect(&mut nodes, 0, 1);
    assert_eq!(nodes[0].replica.connected_peers(), [nodes[1].id]);
    net.connect(&mut nodes, 0, 1);
    assert_eq!(nodes[0].replica.connected_peers(), [nodes[1].id]);
    net.disconnect(&mut nodes, 0, 1);
    assert!(nodes[0].replica.connected_peers().is_empty());
    assert!(nodes[1].replica.connected_peers().is_empty());
}

#[test]
fn should_refuse_updates_the_document_rejects_and_stay_usable() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    let session = net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);
    let version = nodes[0].replica.local().doc.version();
    let garbage = Message::Updates {
        version,
        updates: vec![1, 2, 3, 4],
    };
    let result = nodes[1].replica.on_message(session, garbage);
    assert!(matches!(result, Err(SyncError::Doc(_))), "{result:?}");
    nodes[1].write("later.md", "still works\n");
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[0].files()["later.md"], "still works\n");
}

#[test]
fn should_refuse_a_version_that_does_not_decode() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    let session = net.connect(&mut nodes, 0, 1);
    let result = nodes[1]
        .replica
        .on_message(session, Message::Version(vec![0xff; 3]));
    assert!(matches!(result, Err(SyncError::Doc(_))), "{result:?}");
}

#[test]
fn should_refuse_a_hello_after_the_handshake_and_ignore_unknown_sessions() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    let session = net.connect(&mut nodes, 0, 1);
    let hello = Message::Hello {
        version: 1,
        proof: [0; 32],
    };
    let result = nodes[1].replica.on_message(session, hello.clone());
    assert!(matches!(result, Err(SyncError::Protocol(_))), "{result:?}");
    assert!(nodes[1].replica.on_message(99, hello).unwrap().is_empty());
    let version = Message::Version(nodes[0].replica.local().doc.version());
    assert!(nodes[1].replica.on_message(99, version).unwrap().is_empty());
}

#[test]
fn should_store_at_most_sixty_four_foreign_peers_and_never_its_own_id() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    let session = net.connect(&mut nodes, 0, 1);
    let own = nodes[1].id;
    let mut ids = vec![own];
    ids.extend((10..110u8).map(|n| [n; 32]));
    nodes[1]
        .replica
        .on_message(session, Message::Peers(ids))
        .unwrap();
    let stored = nodes[1].replica.local().state.peers().unwrap();
    // The 64 of the message, plus the device that opened the session, which is recorded on its own.
    assert_eq!(stored.len(), MAX_PEERS_PER_MESSAGE + 1);
    assert!(
        stored
            .iter()
            .all(|peer| from_hex32(&peer.id) != Some(own) && peer.addrs.is_empty())
    );
}

#[test]
fn should_recover_after_a_failed_round_once_the_session_is_reopened() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    let session = net.connect(&mut nodes, 0, 1);
    net.settle(&mut nodes);
    nodes[0].write("a.md", "from a\n");
    let outgoing = nodes[0].replica.reconcile().unwrap();
    let updates = outgoing
        .into_iter()
        .find(|out| matches!(out.message, Message::Updates { .. }));
    let marker = nodes[1].vault_dir.path().join(".secondbrain");
    std::fs::remove_dir_all(&marker).unwrap();
    let failed = nodes[1]
        .replica
        .on_message(session, updates.unwrap().message);
    assert!(
        matches!(failed, Err(SyncError::VaultMarkerMissing(_))),
        "{failed:?}"
    );
    std::fs::create_dir_all(&marker).unwrap();
    net.disconnect(&mut nodes, 0, 1);
    net.connect(&mut nodes, 0, 1);
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[1].files()["a.md"], "from a\n");
    assert_converged(&mut nodes);
}

#[test]
fn should_settle_three_notes_created_at_one_path() {
    let mut nodes = vec![node(1), node(2), node(3)];
    let mut net = Net::new();
    for (i, text) in ["A\n", "B\n", "C\n"].iter().enumerate() {
        nodes[i].write("n.md", text);
        net.reconcile(&mut nodes, i);
    }
    net.connect(&mut nodes, 0, 1);
    net.connect(&mut nodes, 1, 2);
    sync(&mut net, &mut nodes);
    sync(&mut net, &mut nodes);
    let files = nodes[0].files();
    assert_eq!(files.len(), 3, "{files:?}");
    let copies = files
        .keys()
        .filter(|name| is_conflict_name(name, "n"))
        .count();
    assert!(files.contains_key("n.md") && copies == 2, "{files:?}");
    let mut texts: Vec<&str> = files.values().map(String::as_str).collect();
    texts.sort_unstable();
    assert_eq!(texts, ["A\n", "B\n", "C\n"]);
    assert_converged(&mut nodes);
}

#[test]
fn should_keep_crlf_and_missing_final_newline_exactly() {
    let mut nodes = vec![node(1), node(2)];
    let mut net = Net::new();
    net.connect(&mut nodes, 0, 1);
    nodes[0].write("crlf.md", "a\r\nb\r\n");
    nodes[0].write("bare.md", "no newline at end");
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[1].files()["crlf.md"], "a\r\nb\r\n");
    assert_eq!(nodes[1].files()["bare.md"], "no newline at end");
    nodes[1].write("crlf.md", "a\r\nb\r\nc\r\n");
    nodes[1].write("bare.md", "no newline at end\nmore");
    sync(&mut net, &mut nodes);
    assert_eq!(nodes[0].files(), nodes[1].files());
    assert_eq!(nodes[0].files()["crlf.md"], "a\r\nb\r\nc\r\n");
    assert_eq!(nodes[0].files()["bare.md"], "no newline at end\nmore");
    assert_converged(&mut nodes);
}

/// Rewrites one line (0-based) of `n.md` on a node.
fn replace_line(node: &Node, line: usize, text: &str) {
    let current = node.files()["n.md"].clone();
    let mut lines: Vec<&str> = current.lines().collect();
    lines[line] = text;
    node.write("n.md", &format!("{}\n", lines.join("\n")));
}

#[test]
fn should_not_loop_when_both_sides_change_the_same_note_repeatedly() {
    let (mut net, mut nodes) = shared_pair("n.md", "l1\nl2\nl3\nl4\nl5\n");
    for round in 0..5 {
        replace_line(&nodes[0], 0, &format!("A{round}"));
        replace_line(&nodes[1], 4, &format!("B{round}"));
        sync(&mut net, &mut nodes);
        let text = nodes[0].files()["n.md"].clone();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(
            (lines[0], lines[4]),
            (format!("A{round}").as_str(), format!("B{round}").as_str())
        );
        assert_converged(&mut nodes);
    }
    assert!(
        net.deliveries.len() < 200,
        "{} deliveries",
        net.deliveries.len()
    );
}
