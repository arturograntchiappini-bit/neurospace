//! The link graph of a vault: whole, or around one note (spec 6.3).

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use serde::Serialize;

use crate::index::{Index, normalize_target};
use crate::markdown::Link;
use crate::path::{NoteKey, NotePath};

/// Deepest neighbourhood `graph_around` will compute.
pub const MAX_GRAPH_DEPTH: u8 = 5;
const GHOST_PREFIX: char = '?';

/// A note, or a ghost standing for a link target that matches no note.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphNode {
    /// Note path, or `?` plus the normalised target for a ghost.
    pub id: String,
    /// File stem, or the target as first written for a ghost.
    pub label: String,
    /// False for a ghost.
    pub exists: bool,
}

/// A directed link between two nodes, by node id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GraphEdge {
    /// Id of the linking note.
    pub from: String,
    /// Id of the linked note or ghost.
    pub to: String,
}

/// Nodes and edges, in a fixed order.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Graph {
    /// Notes ordered by key, then ghosts ordered by id.
    pub nodes: Vec<GraphNode>,
    /// Edges ordered bytewise by `(from, to)`; one per pair, no self-links.
    pub edges: Vec<GraphEdge>,
}

impl Index {
    /// The link graph of every note, with a ghost node per distinct unresolved target.
    pub fn graph(&self) -> Graph {
        let mut ghosts: BTreeMap<String, String> = BTreeMap::new();
        let mut edges: BTreeSet<(String, String)> = BTreeSet::new();
        for entry in self.notes() {
            for link in &entry.links {
                if let Some(to) = self.target_id(link, &entry.path, &mut ghosts) {
                    edges.insert((entry.path.as_str().to_owned(), to));
                }
            }
        }
        let notes = self.notes().map(|entry| GraphNode {
            id: entry.path.as_str().to_owned(),
            label: entry.path.stem().to_owned(),
            exists: true,
        });
        let ghosts = ghosts.into_iter().map(|(id, label)| GraphNode {
            id,
            label,
            exists: false,
        });
        Graph {
            nodes: notes.chain(ghosts).collect(),
            edges: edges
                .into_iter()
                .map(|(from, to)| GraphEdge { from, to })
                .collect(),
        }
    }

    /// Id of the node a link points to; registers a ghost when unresolved. `None` for a self-link
    /// or a target that normalises to nothing.
    fn target_id(
        &self,
        link: &Link,
        from: &NotePath,
        ghosts: &mut BTreeMap<String, String>,
    ) -> Option<String> {
        let Some(target) = self.resolve(&link.target, from) else {
            let normalized = normalize_target(&link.target);
            if normalized.is_empty() {
                return None;
            }
            let id = format!("{GHOST_PREFIX}{normalized}");
            ghosts
                .entry(id.clone())
                .or_insert_with(|| link.target.clone());
            return Some(id);
        };
        (target != from).then(|| target.as_str().to_owned())
    }

    /// The graph within `depth` steps of a note, edges taken as undirected; `None` for an unknown note.
    /// `depth` is clamped to `1..=MAX_GRAPH_DEPTH`.
    pub fn graph_around(&self, key: &NoteKey, depth: u8) -> Option<Graph> {
        let start = self.get(key)?.path.as_str();
        let graph = self.graph();
        let kept = ids_within(&graph.edges, start, depth.clamp(1, MAX_GRAPH_DEPTH));
        Some(Graph {
            nodes: graph
                .nodes
                .into_iter()
                .filter(|node| kept.contains(&node.id))
                .collect(),
            edges: graph
                .edges
                .into_iter()
                .filter(|edge| kept.contains(&edge.from) && kept.contains(&edge.to))
                .collect(),
        })
    }
}

/// Breadth-first search from `start`, ignoring edge direction.
fn ids_within(edges: &[GraphEdge], start: &str, depth: u8) -> HashSet<String> {
    let mut neighbours: HashMap<&str, Vec<&str>> = HashMap::new();
    for edge in edges {
        neighbours.entry(&edge.from).or_default().push(&edge.to);
        neighbours.entry(&edge.to).or_default().push(&edge.from);
    }
    let mut seen = HashSet::from([start]);
    let mut frontier = vec![start];
    for _ in 0..depth {
        let mut next = Vec::new();
        for id in frontier {
            for &neighbour in neighbours.get(id).into_iter().flatten() {
                if seen.insert(neighbour) {
                    next.push(neighbour);
                }
            }
        }
        frontier = next;
    }
    seen.into_iter().map(str::to_owned).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::path::NotePath;

    fn fixture() -> Index {
        let n = |p: &str, c: &str| (NotePath::parse(p).unwrap(), c.to_string());
        Index::from_notes(vec![
            n(
                "Home.md",
                "[[Idea]] [[projects/Plan]] [[Missing Note]] [[idea|again]]",
            ),
            n("projects/Idea.md", "#work [[Home]]"),
            n("projects/Plan.md", "[[Idea]] [[../Home]]"),
            n("archive/Idea.md", "#work #old"),
            n("archive/deep/Idea.md", ""),
        ])
    }

    fn p(s: &str) -> NotePath {
        NotePath::parse(s).unwrap()
    }

    #[test]
    #[allow(clippy::too_many_lines)] // the brief's test, verbatim; rustfmt expands it past 30 lines
    fn should_build_whole_graph_with_ghost_nodes() {
        let g = fixture().graph();
        let ids: Vec<_> = g
            .nodes
            .iter()
            .map(|n| (n.id.as_str(), n.label.as_str(), n.exists))
            .collect();
        assert_eq!(
            ids,
            [
                ("archive/deep/Idea.md", "Idea", true),
                ("archive/Idea.md", "Idea", true),
                ("Home.md", "Home", true),
                ("projects/Idea.md", "Idea", true),
                ("projects/Plan.md", "Plan", true),
                ("?missing note", "Missing Note", false),
            ]
        );
        let edges: Vec<_> = g
            .edges
            .iter()
            .map(|e| (e.from.as_str(), e.to.as_str()))
            .collect();
        assert_eq!(
            edges,
            [
                ("Home.md", "?missing note"),
                ("Home.md", "archive/Idea.md"),
                ("Home.md", "projects/Plan.md"),
                ("projects/Idea.md", "Home.md"),
                ("projects/Plan.md", "Home.md"),
                ("projects/Plan.md", "projects/Idea.md"),
            ]
        );
    }

    #[test]
    fn should_ignore_self_links_and_duplicates() {
        let ix = Index::from_notes(vec![
            (p("a.md"), "[[a]] [[b]] [[b]] [[B]]".into()),
            (p("b.md"), String::new()),
        ]);
        let g = ix.graph();
        assert_eq!(
            g.edges,
            vec![GraphEdge {
                from: "a.md".into(),
                to: "b.md".into()
            }]
        );
    }

    #[test]
    fn should_return_neighbourhood_by_depth() {
        let ix = fixture();
        let ids = |g: Graph| g.nodes.into_iter().map(|n| n.id).collect::<Vec<_>>();
        let near = ix.graph_around(&p("archive/Idea.md").key(), 1).unwrap();
        assert_eq!(
            near.edges,
            vec![GraphEdge {
                from: "Home.md".into(),
                to: "archive/Idea.md".into()
            }]
        );
        assert_eq!(ids(near), ["archive/Idea.md", "Home.md"]);
        let wide = ix.graph_around(&p("archive/Idea.md").key(), 2).unwrap();
        assert_eq!(
            ids(wide),
            [
                "archive/Idea.md",
                "Home.md",
                "projects/Idea.md",
                "projects/Plan.md",
                "?missing note"
            ]
        );
        let alone = ix
            .graph_around(&p("archive/deep/Idea.md").key(), 9)
            .unwrap();
        assert_eq!(ids(alone), ["archive/deep/Idea.md"]);
        assert!(ix.graph_around(&p("nope.md").key(), 1).is_none());
    }

    #[test]
    fn should_clamp_depth_to_the_documented_range() {
        let chain = (0..=MAX_GRAPH_DEPTH + 2)
            .map(|i| (p(&format!("n{i}.md")), format!("[[n{}]]", i + 1)))
            .collect();
        let ix = Index::from_notes(chain);
        let ids = |depth: u8| {
            let graph = ix.graph_around(&p("n0.md").key(), depth).unwrap();
            graph.nodes.into_iter().map(|n| n.id).collect::<Vec<_>>()
        };
        assert_eq!(ids(0), ids(1));
        assert_eq!(ids(1), ["n0.md", "n1.md"]);
        assert_eq!(ids(u8::MAX), ids(MAX_GRAPH_DEPTH));
        assert_eq!(ids(MAX_GRAPH_DEPTH).len(), usize::from(MAX_GRAPH_DEPTH) + 1);
    }
}
