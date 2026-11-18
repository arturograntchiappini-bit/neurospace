//! Pan and zoom of the graph pane as plain arithmetic, with no UI in sight.
//!
//! A world point `p` is drawn at `centre + pan + p * zoom`, where `centre` is the middle of the
//! pane.

/// Smallest zoom: low enough for «Fit graph» to show every node of a vault of several thousand
/// notes (the simplified layout spreads them over about 12 000 world units).
pub const MIN_ZOOM: f32 = 0.05;
/// Largest zoom.
pub const MAX_ZOOM: f32 = 5.0;
/// Free space left around the graph by `Viewport::fit`.
const FIT_MARGIN: f32 = 24.0;
/// `Viewport::fit` never zooms in beyond this, so a lone note stays small.
const FIT_MAX_ZOOM: f32 = 1.0;

type Point = [f32; 2];

/// The smallest and the largest corner of a box.
pub type Bounds = (Point, Point);

/// Where the graph is looked at from.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    /// Offset of the world origin from the middle of the pane, in screen units.
    pub pan: Point,
    /// Screen units per world unit.
    pub zoom: f32,
}

impl Default for Viewport {
    fn default() -> Viewport {
        Viewport {
            pan: [0.0; 2],
            zoom: 1.0,
        }
    }
}

impl Viewport {
    /// Where the world point `p` is on screen, for a pane whose middle is `centre`.
    pub fn to_screen(&self, centre: Point, p: Point) -> Point {
        [0, 1].map(|axis| centre[axis] + self.pan[axis] + p[axis] * self.zoom)
    }

    /// The world point drawn at the screen point `at`; the inverse of `to_screen`.
    pub fn to_world(&self, centre: Point, at: Point) -> Point {
        [0, 1].map(|axis| (at[axis] - centre[axis] - self.pan[axis]) / self.zoom)
    }

    /// Moves the view by `delta` screen units.
    pub fn pan_by(&mut self, delta: Point) {
        self.pan = [0, 1].map(|axis| self.pan[axis] + delta[axis]);
    }

    /// Multiplies the zoom by `factor` (within the allowed range), keeping the world point under
    /// `anchor`, a screen point relative to the middle of the pane, where it is.
    pub fn zoom_about(&mut self, anchor: Point, factor: f32) {
        let zoom = (self.zoom * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let ratio = zoom / self.zoom;
        self.pan = [0, 1].map(|axis| anchor[axis] - (anchor[axis] - self.pan[axis]) * ratio);
        self.zoom = zoom;
    }

    /// Looks at the middle of `bounds` from as close as lets all of it show in a pane of size
    /// `pane`; the default view for no bounds.
    pub fn fit(&mut self, bounds: Option<Bounds>, pane: Point) {
        let Some((low, high)) = bounds else {
            *self = Viewport::default();
            return;
        };
        let room = |axis: usize| (pane[axis] - 2.0 * FIT_MARGIN).max(1.0);
        let extent = |axis: usize| (high[axis] - low[axis]).max(1.0);
        let fitting = (room(0) / extent(0)).min(room(1) / extent(1));
        self.zoom = fitting.clamp(MIN_ZOOM, FIT_MAX_ZOOM);
        self.pan = [0, 1].map(|axis| -(low[axis] + high[axis]) / 2.0 * self.zoom);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CENTRE: [f32; 2] = [400.0, 300.0];

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn should_round_trip_between_world_and_screen() {
        let view = Viewport {
            pan: [-37.5, 120.0],
            zoom: 2.5,
        };
        for world in [[0.0, 0.0], [12.0, -80.0], [-300.0, 450.5]] {
            let screen = view.to_screen(CENTRE, world);
            assert!(close(view.to_world(CENTRE, screen), world), "{world:?}");
        }
        assert_eq!(Viewport::default().to_screen(CENTRE, [0.0, 0.0]), CENTRE);
    }

    #[test]
    fn should_keep_the_point_under_the_cursor_when_zooming() {
        let mut view = Viewport {
            pan: [20.0, -10.0],
            zoom: 1.0,
        };
        let cursor = [550.0, 380.0];
        let anchor = [cursor[0] - CENTRE[0], cursor[1] - CENTRE[1]];
        let world = view.to_world(CENTRE, cursor);
        view.zoom_about(anchor, 2.0);
        assert_eq!(view.zoom, 2.0);
        assert!(close(view.to_screen(CENTRE, world), cursor));
    }

    #[test]
    fn should_clamp_zoom_without_moving_the_point_under_the_cursor() {
        let mut view = Viewport::default();
        let anchor = [100.0, 50.0];
        let world = view.to_world(CENTRE, [500.0, 350.0]);
        view.zoom_about(anchor, 1000.0);
        assert_eq!(view.zoom, MAX_ZOOM);
        assert!(close(view.to_screen(CENTRE, world), [500.0, 350.0]));
        view.zoom_about(anchor, 1e-6);
        assert_eq!(view.zoom, MIN_ZOOM);
    }

    #[test]
    fn should_fit_a_box_inside_the_pane() {
        let mut view = Viewport {
            pan: [5000.0, 5000.0],
            zoom: 4.0,
        };
        let bounds = ([-900.0, -100.0], [1500.0, 300.0]);
        view.fit(Some(bounds), [600.0, 400.0]);
        for corner in [
            bounds.0,
            bounds.1,
            [bounds.0[0], bounds.1[1]],
            [bounds.1[0], bounds.0[1]],
        ] {
            let [x, y] = view.to_screen([300.0, 200.0], corner);
            assert!(
                (0.0..=600.0).contains(&x) && (0.0..=400.0).contains(&y),
                "{x} {y}"
            );
        }
    }

    #[test]
    fn should_fit_the_whole_of_a_very_large_graph_when_fitting() {
        // About 3000 notes spread over roughly 12 000 world units in a 900 by 600 pane.
        let mut view = Viewport::default();
        let bounds = ([-6000.0, -6000.0], [6000.0, 6000.0]);
        view.fit(Some(bounds), [900.0, 600.0]);
        for corner in [bounds.0, bounds.1] {
            let [x, y] = view.to_screen([450.0, 300.0], corner);
            assert!(
                (0.0..=900.0).contains(&x) && (0.0..=600.0).contains(&y),
                "{x} {y}"
            );
        }
    }

    #[test]
    fn should_reset_when_fitting_nothing_and_not_zoom_in_on_one_node() {
        let mut view = Viewport {
            pan: [9.0, 9.0],
            zoom: 3.0,
        };
        view.fit(None, [600.0, 400.0]);
        assert_eq!(view, Viewport::default());
        view.fit(Some(([-4.0, -4.0], [4.0, 4.0])), [600.0, 400.0]);
        assert_eq!(view.zoom, 1.0);
    }
}
