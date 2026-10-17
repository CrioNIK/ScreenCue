//! Window-relative placement. No screenshots, recognition, or input injection.
use crate::model::{Hint, Rect};
use crate::platform::AnchorState;

pub fn translated(base: &Hint, origin: AnchorState, current: AnchorState) -> Hint {
    let ratio = scale(current.scale) / scale(origin.scale);
    let x = |value: i32| coordinate(current.x, value, origin.x, ratio);
    let y = |value: i32| coordinate(current.y, value, origin.y, ratio);
    let mut hint = base.clone();
    hint.x = x(base.x);
    hint.y = y(base.y);
    for point in &mut hint.points {
        point.x = x(point.x);
        point.y = y(point.y);
    }
    if hint.shape() != "polygon" {
        hint.width = Some(
            (base.width.unwrap_or(16) as f64 * ratio)
                .round()
                .clamp(1., 4000.) as u32,
        );
        hint.height = Some(
            (base.height.unwrap_or(16) as f64 * ratio)
                .round()
                .clamp(1., 4000.) as u32,
        );
    }
    hint
}

fn scale(value: f64) -> f64 {
    if value.is_finite() && value > 0. {
        value
    } else {
        1.
    }
}

fn coordinate(current: i32, value: i32, origin: i32, ratio: f64) -> i32 {
    (current as f64 + (value as f64 - origin as f64) * ratio)
        .round()
        .clamp(i32::MIN as f64, i32::MAX as f64) as i32
}

pub fn resized(origin: AnchorState, current: AnchorState) -> bool {
    let ratio = scale(current.scale) / scale(origin.scale);
    (current.width as f64 - origin.width as f64 * ratio).abs() > 2.
        || (current.height as f64 - origin.height as f64 * ratio).abs() > 2.
}

pub fn fits(hint: &Hint, monitor: Rect) -> bool {
    let b = hint.bounds();
    b.x >= monitor.x
        && b.y >= monitor.y
        && b.right() <= monitor.right()
        && b.bottom() <= monitor.bottom()
}

pub fn nearest(hint: &Hint, monitors: &[(Rect, f32)]) -> Option<(Rect, f32)> {
    let (x, y) = hint.anchor();
    monitors.iter().copied().min_by(|(a, _), (b, _)| {
        let distance = |r: Rect| {
            let dx = x - x.clamp(r.x, r.right());
            let dy = y - y.clamp(r.y, r.bottom());
            dx * dx + dy * dy
        };
        distance(*a).total_cmp(&distance(*b))
    })
}

/// Keep a truthful edge indicator when the real target cannot be shown.
/// No contour is drawn around a substitute control at the screen edge.
pub fn edge_hint(hint: &Hint, monitor: Rect, scale: f32, note: &str) -> Hint {
    let (x, y) = hint.anchor();
    let mx = (32. * scale).min(monitor.w / 3.);
    let my = (32. * scale).min(monitor.h / 3.);
    let left = monitor.x + mx;
    let right = monitor.right() - mx;
    let top = monitor.y + my;
    let bottom = monitor.bottom() - my;
    let mut px = x.clamp(left, right);
    let mut py = y.clamp(top, bottom);
    // Minimized or resized targets may still have their last center on screen.
    // In that case choose the nearest edge, rather than pointing at stale UI.
    if x >= left && x <= right && y >= top && y <= bottom {
        let distances = [x - left, right - x, y - top, bottom - y];
        let edge = (0..4)
            .min_by(|a, b| distances[*a].total_cmp(&distances[*b]))
            .unwrap();
        match edge {
            0 => px = left,
            1 => px = right,
            2 => py = top,
            _ => py = bottom,
        }
    }
    let mut display = hint.clone();
    display.x = px.round() as i32;
    display.y = py.round() as i32;
    display.style = "arrow".into();
    display.shape = Some("ellipse".into());
    display.width = Some(1);
    display.height = Some(1);
    display.points.clear();
    display.arrow = Some(true);
    display.target = false;
    display.title.clear();
    display.text = note.into();
    display.step = 1;
    display
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn state(x: i32, y: i32, scale: f64) -> AnchorState {
        AnchorState {
            x,
            y,
            width: (800. * scale) as u32,
            height: (600. * scale) as u32,
            scale,
            visible: true,
            destroyed: false,
        }
    }
    #[test]
    fn follows_original_window_and_dpi_without_accumulating_drift() {
        let hint = Hint::parse(json!({"x":250,"y":300,"width":80,"height":40})).unwrap();
        let origin = state(100, 100, 1.);
        let moved = translated(&hint, origin, state(-1000, 200, 2.));
        assert_eq!(
            (moved.x, moved.y, moved.width, moved.height),
            (-700, 600, Some(160), Some(80))
        );
        assert!(!resized(origin, state(-1000, 200, 2.)));
        let back = translated(&hint, origin, origin);
        assert_eq!((back.x, back.y), (hint.x, hint.y));
    }
    #[test]
    fn polygon_moves_as_a_whole_and_resize_is_not_mistaken_for_translation() {
        let hint = Hint::parse(json!({"x":30,"y":30,"shape":"polygon","points":[{"x":10,"y":10},{"x":90,"y":10},{"x":50,"y":90}]})).unwrap();
        let origin = state(0, 0, 1.);
        let moved = translated(&hint, origin, state(-200, 50, 1.));
        assert_eq!((moved.points[0].x, moved.points[0].y), (-190, 60));
        let mut changed = origin;
        changed.width += 50;
        assert!(resized(origin, changed));
    }
    #[test]
    fn offscreen_and_minimized_markers_stay_on_negative_origin_monitor() {
        let monitor = Rect {
            x: -1920.,
            y: -120.,
            w: 1920.,
            h: 1080.,
        };
        for (x, y) in [(-3000, -1000), (-100, 300), (9000, 9000)] {
            let hint = Hint::parse(json!({"x":x,"y":y})).unwrap();
            let marker = edge_hint(&hint, monitor, 1., "Target offscreen");
            assert!(fits(&marker, monitor));
            assert!(!marker.target);
            assert_eq!(marker.text, "Target offscreen");
        }
    }
}
