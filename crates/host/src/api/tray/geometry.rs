//! Pure placement math for tray-attached windows. All values are physical
//! pixels in virtual-desktop coordinates.

/// Axis-aligned rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Bounds {
    pub(crate) x: f64,
    pub(crate) y: f64,
    pub(crate) width: f64,
    pub(crate) height: f64,
}

impl Bounds {
    fn right(&self) -> f64 {
        self.x + self.width
    }

    fn bottom(&self) -> f64 {
        self.y + self.height
    }

    fn center_x(&self) -> f64 {
        self.x + self.width / 2.0
    }

    fn center_y(&self) -> f64 {
        self.y + self.height / 2.0
    }

    pub(crate) fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// Screen edge the tray icon sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

/// Edge of `monitor` the taskbar / menu bar holding `tray` is docked to.
/// The icon lies outside the work area when the OS reports one (Windows,
/// macOS); otherwise the closest monitor edge wins, top/bottom first.
pub(crate) fn tray_edge(tray: &Bounds, monitor: &Bounds, work_area: &Bounds) -> Edge {
    let (cx, cy) = (tray.center_x(), tray.center_y());
    if cy >= work_area.bottom() {
        return Edge::Bottom;
    }
    if cy < work_area.y {
        return Edge::Top;
    }
    if cx < work_area.x {
        return Edge::Left;
    }
    if cx >= work_area.right() {
        return Edge::Right;
    }
    let distances = [
        (cy - monitor.y, Edge::Top),
        (monitor.bottom() - cy, Edge::Bottom),
        (cx - monitor.x, Edge::Left),
        (monitor.right() - cx, Edge::Right),
    ];
    distances
        .iter()
        .copied()
        .min_by(|left, right| left.0.total_cmp(&right.0))
        .map_or(Edge::Bottom, |(_, edge)| edge)
}

/// Top-left corner for a `width` x `height` window next to the tray icon:
/// below it on a top bar (macOS, most Linux panels), above it on a bottom
/// taskbar (Windows), beside it on a vertical taskbar. Centred on the icon
/// and clamped inside `work_area`, `gap` pixels away from the bar.
pub(crate) fn position_near_tray(
    tray: &Bounds,
    width: f64,
    height: f64,
    monitor: &Bounds,
    work_area: &Bounds,
    gap: f64,
) -> (f64, f64) {
    let (x, y) = match tray_edge(tray, monitor, work_area) {
        Edge::Top => (
            tray.center_x() - width / 2.0,
            tray.bottom().max(work_area.y) + gap,
        ),
        Edge::Bottom => (
            tray.center_x() - width / 2.0,
            tray.y.min(work_area.bottom()) - height - gap,
        ),
        Edge::Left => (
            tray.right().max(work_area.x) + gap,
            tray.center_y() - height / 2.0,
        ),
        Edge::Right => (
            tray.x.min(work_area.right()) - width - gap,
            tray.center_y() - height / 2.0,
        ),
    };
    (
        clamp(x, work_area.x, work_area.right() - width),
        clamp(y, work_area.y, work_area.bottom() - height),
    )
}

/// Clamp `value` into `[min, max]`; a window larger than the area sticks to
/// `min` (its top-left stays visible).
fn clamp(value: f64, min: f64, max: f64) -> f64 {
    value.min(max).max(min)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bounds(x: f64, y: f64, width: f64, height: f64) -> Bounds {
        Bounds {
            x,
            y,
            width,
            height,
        }
    }

    const MONITOR: Bounds = Bounds {
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    };

    #[test]
    fn windows_bottom_taskbar_places_window_above_the_icon() {
        let work = bounds(0.0, 0.0, 1920.0, 1032.0);
        let tray = bounds(1700.0, 1040.0, 24.0, 32.0);
        assert_eq!(tray_edge(&tray, &MONITOR, &work), Edge::Bottom);
        let (x, y) = position_near_tray(&tray, 300.0, 400.0, &MONITOR, &work, 8.0);
        assert!((x - (1712.0 - 150.0)).abs() < f64::EPSILON);
        assert!((y - (1032.0 - 400.0 - 8.0)).abs() < f64::EPSILON);
    }

    #[test]
    fn icon_near_the_screen_corner_is_clamped_into_the_work_area() {
        let work = bounds(0.0, 0.0, 1920.0, 1032.0);
        let tray = bounds(1890.0, 1040.0, 24.0, 32.0);
        let (x, _) = position_near_tray(&tray, 300.0, 400.0, &MONITOR, &work, 8.0);
        assert!((x - 1620.0).abs() < f64::EPSILON);
    }

    #[test]
    fn macos_menu_bar_places_window_below_the_icon() {
        let work = bounds(0.0, 25.0, 1920.0, 1055.0);
        let tray = bounds(1500.0, 0.0, 22.0, 24.0);
        assert_eq!(tray_edge(&tray, &MONITOR, &work), Edge::Top);
        let (x, y) = position_near_tray(&tray, 300.0, 400.0, &MONITOR, &work, 4.0);
        assert!((x - 1361.0).abs() < f64::EPSILON);
        assert!((y - 29.0).abs() < f64::EPSILON);
    }

    #[test]
    fn vertical_taskbars_place_the_window_beside_the_icon() {
        let left_work = bounds(48.0, 0.0, 1872.0, 1080.0);
        let tray = bounds(8.0, 900.0, 32.0, 32.0);
        assert_eq!(tray_edge(&tray, &MONITOR, &left_work), Edge::Left);
        let (x, y) = position_near_tray(&tray, 300.0, 400.0, &MONITOR, &left_work, 8.0);
        assert!((x - 56.0).abs() < f64::EPSILON);
        assert!(
            (y - 680.0).abs() < f64::EPSILON,
            "clamped above the work-area bottom"
        );

        let right_work = bounds(0.0, 0.0, 1872.0, 1080.0);
        let tray = bounds(1880.0, 20.0, 32.0, 32.0);
        assert_eq!(tray_edge(&tray, &MONITOR, &right_work), Edge::Right);
        let (x, y) = position_near_tray(&tray, 300.0, 400.0, &MONITOR, &right_work, 8.0);
        assert!((x - 1564.0).abs() < f64::EPSILON);
        assert!(
            y.abs() < f64::EPSILON,
            "clamped to the top of the work area"
        );
    }

    #[test]
    fn without_a_work_area_the_nearest_monitor_edge_wins() {
        // Linux: no work area and only the click point as the anchor.
        let anchor = bounds(1800.0, 10.0, 0.0, 0.0);
        assert_eq!(tray_edge(&anchor, &MONITOR, &MONITOR), Edge::Top);
        let anchor = bounds(900.0, 1075.0, 0.0, 0.0);
        assert_eq!(tray_edge(&anchor, &MONITOR, &MONITOR), Edge::Bottom);
    }

    #[test]
    fn contains_is_half_open() {
        assert!(MONITOR.contains(0.0, 0.0));
        assert!(!MONITOR.contains(1920.0, 10.0));
    }
}
