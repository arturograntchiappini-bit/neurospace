//! Force-directed layout of the note graph, with no UI in sight (spec 9.4).
//!
//! Springs pull linked notes together, every pair repels, a weak pull towards the origin keeps
//! unlinked islands from drifting apart, and damping lets it come to rest.

use std::collections::HashMap;
use std::mem;

use sb_core::Graph;

const SPIRAL_SPACING: f32 = 30.0;
const GOLDEN_ANGLE: f32 = 2.39996;
const REPULSION: f32 = 2000.0;
const MIN_DISTANCE_SQUARED: f32 = 1.0;
const SPRING_STIFFNESS: f32 = 0.02;
const SPRING_REST_LENGTH: f32 = 60.0;
const CENTRE_PULL: f32 = 0.002;
const DAMPING: f32 = 0.8;
const MAX_STEP: f32 = 30.0;
const SETTLED_BELOW: f32 = 0.05;
const BASE_RADIUS: f32 = 4.0;
const RADIUS_PER_ROOT_DEGREE: f32 = 2.0;

type Point = [f32; 2];

struct NodeInfo {
    id: String,
    label: String,
    exists: bool,
    neighbours: Vec<usize>,
}

/// Positions of the graph's nodes and the simulation that moves them.
///
/// Node `i` is the `i`-th node by id; indices change when `update` adds or removes nodes.
pub struct Layout {
    nodes: Vec<NodeInfo>,
    positions: Vec<Point>,
    velocities: Vec<Point>,
    pinned: Option<usize>,
    last_move: f32,
}

/// Where node `i` starts: a sunflower spiral, so that no two nodes begin on top of each other.
fn spiral(i: usize) -> Point {
    let radius = SPIRAL_SPACING * (i as f32).sqrt();
    let angle = i as f32 * GOLDEN_ANGLE;
    [radius * angle.cos(), radius * angle.sin()]
}

impl Layout {
    /// A layout of `graph` with every node on its starting spiral position.
    pub fn new(graph: &Graph) -> Layout {
        let mut layout = Layout {
            nodes: Vec::new(),
            positions: Vec::new(),
            velocities: Vec::new(),
            pinned: None,
            last_move: f32::INFINITY,
        };
        layout.update(graph);
        layout
    }

    /// Adopts `graph`: nodes still present keep their position, new ones start on the spiral.
    pub fn update(&mut self, graph: &Graph) {
        let old_ids = mem::take(&mut self.nodes).into_iter().map(|node| node.id);
        let mut known: HashMap<String, Point> =
            old_ids.zip(mem::take(&mut self.positions)).collect();
        let mut sorted: Vec<_> = graph.nodes.iter().collect();
        sorted.sort_by(|a, b| a.id.cmp(&b.id));
        self.positions = sorted
            .iter()
            .enumerate()
            .map(|(i, node)| known.remove(&node.id).unwrap_or_else(|| spiral(i)))
            .collect();
        self.nodes = sorted
            .iter()
            .map(|node| NodeInfo {
                id: node.id.clone(),
                label: node.label.clone(),
                exists: node.exists,
                neighbours: Vec::new(),
            })
            .collect();
        self.velocities = vec![[0.0; 2]; self.nodes.len()];
        self.connect(graph);
        self.pinned = None;
        self.last_move = f32::INFINITY;
    }

    /// Fills in each node's neighbours from the edges, ignoring direction, duplicates and
    /// edges to unknown nodes.
    fn connect(&mut self, graph: &Graph) {
        let by_id: HashMap<&str, usize> = self
            .nodes
            .iter()
            .enumerate()
            .map(|(i, node)| (node.id.as_str(), i))
            .collect();
        let mut pairs: Vec<(usize, usize)> = graph
            .edges
            .iter()
            .filter_map(|edge| {
                Some((
                    *by_id.get(edge.from.as_str())?,
                    *by_id.get(edge.to.as_str())?,
                ))
            })
            .filter(|(a, b)| a != b)
            .map(|(a, b)| (a.min(b), a.max(b)))
            .collect();
        pairs.sort_unstable();
        pairs.dedup();
        for (a, b) in pairs {
            self.nodes[a].neighbours.push(b);
            self.nodes[b].neighbours.push(a);
        }
    }

    /// One step of the simulation. Returns the largest distance any node moved, or would have
    /// moved when it was pinned by `set_position`.
    pub fn step(&mut self) -> f32 {
        let mut force = vec![[0.0; 2]; self.nodes.len()];
        self.repel(&mut force);
        self.attract(&mut force);
        self.last_move = self.advance(&force);
        self.last_move
    }

    /// shortcut: all pairs, quadratic. Fine for a few thousand notes; use Barnes-Hut beyond.
    fn repel(&self, force: &mut [Point]) {
        for (i, a) in self.positions.iter().enumerate() {
            for (j, b) in self.positions.iter().enumerate().skip(i + 1) {
                let (dx, dy) = (a[0] - b[0], a[1] - b[1]);
                let squared = (dx * dx + dy * dy).max(MIN_DISTANCE_SQUARED);
                let push = REPULSION / (squared * squared.sqrt());
                force[i][0] += dx * push;
                force[i][1] += dy * push;
                force[j][0] -= dx * push;
                force[j][1] -= dy * push;
            }
        }
    }

    fn attract(&self, force: &mut [Point]) {
        for (i, node) in self.nodes.iter().enumerate() {
            for &j in node.neighbours.iter().filter(|&&j| j > i) {
                let (dx, dy) = (
                    self.positions[j][0] - self.positions[i][0],
                    self.positions[j][1] - self.positions[i][1],
                );
                let distance = (dx * dx + dy * dy).sqrt().max(1.0);
                let pull = SPRING_STIFFNESS * (distance - SPRING_REST_LENGTH) / distance;
                force[i][0] += dx * pull;
                force[i][1] += dy * pull;
                force[j][0] -= dx * pull;
                force[j][1] -= dy * pull;
            }
        }
    }

    /// Applies the forces with damping and a speed limit; a pinned node stays where it is, for
    /// this step only. Returns the largest movement.
    fn advance(&mut self, force: &[Point]) -> f32 {
        let pinned = self.pinned.take();
        let mut largest = 0.0_f32;
        for (i, push) in force.iter().enumerate() {
            let position = &mut self.positions[i];
            let velocity = &mut self.velocities[i];
            let wanted = [
                (velocity[0] + push[0] - CENTRE_PULL * position[0]) * DAMPING,
                (velocity[1] + push[1] - CENTRE_PULL * position[1]) * DAMPING,
            ];
            let speed = wanted[0].hypot(wanted[1]);
            let scale = if speed > MAX_STEP {
                MAX_STEP / speed
            } else {
                1.0
            };
            largest = largest.max(speed * scale);
            *velocity = if pinned == Some(i) {
                [0.0; 2]
            } else {
                [wanted[0] * scale, wanted[1] * scale]
            };
            position[0] += velocity[0];
            position[1] += velocity[1];
        }
        largest
    }

    /// True when the last step moved every node less than a twentieth of a unit.
    pub fn is_settled(&self) -> bool {
        self.last_move < SETTLED_BELOW
    }

    /// Number of nodes.
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// True when the graph has no nodes.
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    /// Where node `i` is.
    pub fn position(&self, i: usize) -> Point {
        self.positions[i]
    }

    /// Puts node `i` at `p` (a drag): the node stays there for the next step and the layout wakes.
    pub fn set_position(&mut self, i: usize, p: Point) {
        self.positions[i] = p;
        self.velocities[i] = [0.0; 2];
        self.pinned = Some(i);
        self.last_move = f32::INFINITY;
    }

    /// Radius of node `i` in layout units, growing with the square root of its link count.
    pub fn radius(&self, i: usize) -> f32 {
        BASE_RADIUS + RADIUS_PER_ROOT_DEGREE * (self.nodes[i].neighbours.len() as f32).sqrt()
    }

    /// The topmost (last drawn) node whose circle contains `p`.
    pub fn node_at(&self, p: Point) -> Option<usize> {
        (0..self.len()).rev().find(|&i| {
            let [x, y] = self.positions[i];
            let reach = self.radius(i);
            (x - p[0]).powi(2) + (y - p[1]).powi(2) <= reach * reach
        })
    }

    /// The nodes linked to node `i`, in either direction, each once.
    pub fn neighbours(&self, i: usize) -> &[usize] {
        &self.nodes[i].neighbours
    }

    /// Id of node `i`: the note path, or `?` and the would-be key for a ghost.
    pub fn id(&self, i: usize) -> &str {
        &self.nodes[i].id
    }

    /// Text to show next to node `i`.
    pub fn label(&self, i: usize) -> &str {
        &self.nodes[i].label
    }

    /// False for a ghost, which stands for a note that does not exist yet.
    pub fn exists(&self, i: usize) -> bool {
        self.nodes[i].exists
    }
}

#[cfg(test)]
mod tests {
    use sb_core::{Graph, GraphEdge, GraphNode};

    use super::*;

    fn graph(ids: &[&str], edges: &[(&str, &str)]) -> Graph {
        Graph {
            nodes: ids
                .iter()
                .map(|id| GraphNode {
                    id: (*id).to_owned(),
                    label: id.trim_end_matches(".md").to_owned(),
                    exists: !id.starts_with('?'),
                })
                .collect(),
            edges: edges
                .iter()
                .map(|(from, to)| GraphEdge {
                    from: (*from).to_owned(),
                    to: (*to).to_owned(),
                })
                .collect(),
        }
    }

    fn index_of(layout: &Layout, id: &str) -> usize {
        (0..layout.len()).find(|&i| layout.id(i) == id).unwrap()
    }

    fn ring_graph(size: usize, chords: usize) -> Graph {
        let ids: Vec<String> = (0..size).map(|i| format!("n{i}.md")).collect();
        let mut edges: Vec<(String, String)> = (0..size)
            .map(|i| (ids[i].clone(), ids[(i + 1) % size].clone()))
            .collect();
        edges.extend((0..chords).map(|i| (ids[i].clone(), ids[(i + size / 2) % size].clone())));
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let edge_refs: Vec<(&str, &str)> = edges
            .iter()
            .map(|(a, b)| (a.as_str(), b.as_str()))
            .collect();
        graph(&id_refs, &edge_refs)
    }

    #[test]
    fn should_place_nodes_deterministically() {
        let g = graph(&["b.md", "a.md", "?c"], &[("a.md", "b.md"), ("a.md", "?c")]);
        let (first, second) = (Layout::new(&g), Layout::new(&g));
        assert_eq!(first.len(), 3);
        for i in 0..first.len() {
            assert_eq!(first.position(i), second.position(i));
        }
        assert_eq!(first.id(0), "?c");
        assert_eq!(first.position(0), [0.0, 0.0]);
        let [x, y] = first.position(1);
        let expected = 30.0 * 1.0_f32.sqrt();
        assert!((x - expected * 2.39996_f32.cos()).abs() < 1e-4);
        assert!((y - expected * 2.39996_f32.sin()).abs() < 1e-4);
    }

    #[test]
    fn should_settle_small_graph() {
        let mut layout = Layout::new(&ring_graph(10, 2));
        assert!(!layout.is_settled());
        let steps = (0..2000).take_while(|_| {
            layout.step();
            !layout.is_settled()
        });
        assert!(steps.count() < 2000, "the layout never settled");
        assert!(layout.is_settled());
    }

    #[test]
    fn should_keep_positions_of_known_nodes_on_update() {
        let mut layout = Layout::new(&ring_graph(6, 0));
        for _ in 0..50 {
            layout.step();
        }
        let before: Vec<(String, [f32; 2])> = (0..layout.len())
            .map(|i| (layout.id(i).to_owned(), layout.position(i)))
            .collect();
        let mut bigger = ring_graph(6, 0);
        bigger.nodes.push(GraphNode {
            id: "!new.md".into(),
            label: "new".into(),
            exists: true,
        });
        layout.update(&bigger);
        assert_eq!(layout.len(), 7);
        assert!(!layout.is_settled());
        for (id, position) in before {
            assert_eq!(layout.position(index_of(&layout, &id)), position, "{id}");
        }
        let added = layout.position(index_of(&layout, "!new.md"));
        assert!(added.iter().all(|c| c.is_finite()));
    }

    #[test]
    fn should_size_nodes_by_degree() {
        let g = graph(
            &["hub.md", "a.md", "b.md", "c.md", "d.md", "lonely.md"],
            &[
                ("hub.md", "a.md"),
                ("b.md", "hub.md"),
                ("hub.md", "c.md"),
                ("d.md", "hub.md"),
                ("hub.md", "a.md"),
                ("a.md", "hub.md"),
            ],
        );
        let layout = Layout::new(&g);
        assert_eq!(layout.radius(index_of(&layout, "lonely.md")), 4.0);
        let hub = index_of(&layout, "hub.md");
        assert_eq!(layout.radius(hub), 8.0);
        assert_eq!(layout.neighbours(hub).len(), 4);
    }

    #[test]
    fn should_find_node_under_point() {
        let g = graph(&["a.md", "b.md", "?g"], &[("a.md", "b.md")]);
        let mut layout = Layout::new(&g);
        let a = index_of(&layout, "a.md");
        let b = index_of(&layout, "b.md");
        layout.set_position(a, [100.0, 100.0]);
        layout.set_position(b, [300.0, 100.0]);
        let radius = layout.radius(a);
        assert_eq!(layout.node_at([100.0 + radius - 0.5, 100.0]), Some(a));
        assert_eq!(layout.node_at([100.0 + radius + 0.5, 100.0]), None);
        assert_eq!(layout.node_at([300.0, 100.0]), Some(b));
        layout.set_position(a, [300.0, 100.0]);
        let on_top = layout.node_at([300.0, 100.0]).unwrap();
        assert_eq!(on_top, a.max(b), "the later node is drawn on top");
    }

    #[test]
    fn should_expose_labels_ghosts_and_neighbours() {
        let g = graph(&["a.md", "?ghost"], &[("a.md", "?ghost")]);
        let layout = Layout::new(&g);
        let ghost = index_of(&layout, "?ghost");
        assert!(!layout.exists(ghost));
        assert_eq!(layout.label(ghost), "?ghost");
        let a = index_of(&layout, "a.md");
        assert!(layout.exists(a));
        assert_eq!(layout.label(a), "a");
        assert_eq!(layout.neighbours(a), [ghost]);
        assert_eq!(layout.neighbours(ghost), [a]);
    }

    #[test]
    fn should_keep_dragged_node_in_place_for_one_step() {
        let mut layout = Layout::new(&ring_graph(4, 0));
        layout.set_position(0, [500.0, 500.0]);
        layout.step();
        assert_eq!(layout.position(0), [500.0, 500.0]);
        layout.step();
        assert_ne!(layout.position(0), [500.0, 500.0]);
    }

    #[test]
    fn should_step_ten_thousand_nodes_without_overflow() {
        let ids: Vec<String> = (0..10_000).map(|i| format!("n{i}.md")).collect();
        let id_refs: Vec<&str> = ids.iter().map(String::as_str).collect();
        let mut layout = Layout::new(&graph(&id_refs, &[("n0.md", "n1.md")]));
        layout.step();
        for i in 0..layout.len() {
            assert!(layout.position(i).iter().all(|c| c.is_finite()), "node {i}");
        }
    }
}
