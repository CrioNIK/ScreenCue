//! CPU rendering only: small transparent bitmaps, no capture, animation, or polling.

use crate::model::{Hint, Rect};
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
use fontdue::{Font, FontSettings};
use tiny_skia::{FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, Stroke, Transform};

pub struct Frame {
    pub pixmap: Pixmap,
    pub x: i32,
    pub y: i32,
    /// Physical desktop coordinate, useful for inspection and geometry tests.
    pub arrow_tip: Option<(f32, f32)>,
}

pub struct Renderer {
    font: Font,
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        let font = Font::from_bytes(
            include_bytes!("../assets/IBMPlexMono-Regular.ttf") as &[u8],
            FontSettings::default(),
        )
        .map_err(|error| format!("Cannot load bundled font: {error}"))?;
        Ok(Self { font })
    }

    pub fn render(
        &self,
        hint: &Hint,
        monitor: Rect,
        scale: f32,
        color: [u8; 3],
    ) -> Result<Frame, String> {
        if !scale.is_finite()
            || scale <= 0.0
            || !monitor.w.is_finite()
            || !monitor.h.is_finite()
            || monitor.w < 1.0
            || monitor.h < 1.0
        {
            return Err("Invalid display dimensions or scale".into());
        }
        let scale = scale.clamp(0.5, 4.0);
        let target = hint.bounds();
        let anchor = hint.anchor();
        let margin = 4.0 * scale;
        let padding = 9.0 * scale;
        let label = if hint.step > 1 {
            format!("{} · {}", hint.step, hint.text)
        } else {
            hint.text.clone()
        };
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        let card = if hint.text.is_empty() {
            None
        } else {
            let max_width = (280.0 * scale).min(monitor.w - 2.0 * margin) - 2.0 * padding;
            if max_width < 13.0 * scale {
                return Err("Display is too narrow for the instruction".into());
            }
            layout.reset(&LayoutSettings {
                max_width: Some(max_width),
                line_height: 1.12,
                ..LayoutSettings::default()
            });
            layout.append(&[&self.font], &TextStyle::new(&label, 13.0 * scale, 0));
            let text_width = layout
                .glyphs()
                .iter()
                .map(|glyph| glyph.x + glyph.width as f32)
                .fold(0.0, f32::max)
                .ceil();
            let width = (text_width.max(1.0) + 2.0 * padding).min(monitor.w - 2.0 * margin);
            let height = layout.height().max(13.0 * scale).ceil() + 2.0 * padding;
            if height > monitor.h - 2.0 * margin {
                return Err(
                    "Instruction is too tall for the display; shorten it or remove line breaks"
                        .into(),
                );
            }
            Some(place_card(target, monitor, width, height, scale))
        };

        let arrow = if hint.has_arrow() {
            make_arrow(hint, monitor, card, scale)
        } else {
            None
        };
        let mut bounds = if hint.target { Some(target) } else { None };
        if let Some(card) = card {
            add_bounds(&mut bounds, card);
        }
        if let Some(arrow) = arrow {
            let xs = [arrow.start.0, arrow.tip.0, arrow.wing_a.0, arrow.wing_b.0];
            let ys = [arrow.start.1, arrow.tip.1, arrow.wing_a.1, arrow.wing_b.1];
            let x = xs.into_iter().fold(f32::INFINITY, f32::min);
            let y = ys.into_iter().fold(f32::INFINITY, f32::min);
            add_bounds(
                &mut bounds,
                Rect {
                    x,
                    y,
                    w: xs.into_iter().fold(f32::NEG_INFINITY, f32::max) - x,
                    h: ys.into_iter().fold(f32::NEG_INFINITY, f32::max) - y,
                },
            );
        }
        let bounds = bounds.unwrap_or(Rect {
            x: anchor.0,
            y: anchor.1,
            w: 1.0,
            h: 1.0,
        });
        let x = (bounds.x - margin).floor().max(monitor.x.floor()) as i32;
        let y = (bounds.y - margin).floor().max(monitor.y.floor()) as i32;
        let right = (bounds.right() + margin).ceil().min(monitor.right().ceil());
        let bottom = (bounds.bottom() + margin)
            .ceil()
            .min(monitor.bottom().ceil());
        let width = (right - x as f32).max(1.0) as u32;
        let height = (bottom - y as f32).max(1.0) as u32;
        if width > 8192 || height > 8192 || width as u64 * height as u64 > 20_000_000 {
            return Err("Hint bitmap exceeds the supported dimensions".into());
        }
        let mut pixmap = Pixmap::new(width, height).ok_or("Cannot allocate hint bitmap")?;
        let transform = Transform::from_translate(-(x as f32), -(y as f32));
        let mut paint = Paint::default();
        paint.set_color_rgba8(color[0], color[1], color[2], 255);
        paint.anti_alias = true;
        if hint.target {
            let path = target_path(hint)?;
            let stroke = Stroke {
                width: 1.3 * scale,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, transform, None);
        }
        if let Some(arrow) = arrow {
            let mut builder = PathBuilder::new();
            builder.move_to(arrow.start.0, arrow.start.1);
            builder.line_to(arrow.tip.0, arrow.tip.1);
            builder.move_to(arrow.wing_a.0, arrow.wing_a.1);
            builder.line_to(arrow.tip.0, arrow.tip.1);
            builder.line_to(arrow.wing_b.0, arrow.wing_b.1);
            if let Some(path) = builder.finish() {
                let stroke = Stroke {
                    width: 1.8 * scale,
                    line_cap: LineCap::Round,
                    line_join: LineJoin::Round,
                    ..Stroke::default()
                };
                pixmap.stroke_path(&path, &paint, &stroke, transform, None);
            }
        }
        if let Some(card) = card {
            let path = rounded_rect(card, 7.0 * scale)?;
            let mut background = Paint::default();
            background.set_color_rgba8(20, 23, 29, 231);
            background.anti_alias = true;
            pixmap.fill_path(&path, &background, FillRule::Winding, transform, None);
            for glyph in layout.glyphs() {
                let (metrics, coverage) = self.font.rasterize_config(glyph.key);
                let gx = (card.x + padding + glyph.x - x as f32).round() as i32;
                let gy = (card.y + padding + glyph.y - y as f32).round() as i32;
                blend_glyph(
                    &mut pixmap,
                    gx,
                    gy,
                    metrics.width,
                    metrics.height,
                    &coverage,
                );
            }
        }
        Ok(Frame {
            pixmap,
            x,
            y,
            arrow_tip: arrow.map(|arrow| arrow.tip),
        })
    }
}

fn add_bounds(bounds: &mut Option<Rect>, next: Rect) {
    *bounds = Some(bounds.map_or(next, |previous| previous.union(next)));
}

fn overlap_area(a: Rect, b: Rect) -> f32 {
    (a.right().min(b.right()) - a.x.max(b.x)).max(0.0)
        * (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0)
}

fn place_card(target: Rect, monitor: Rect, width: f32, height: f32, scale: f32) -> Rect {
    let gap = 56.0 * scale;
    let inset = 4.0 * scale;
    let cx = target.x + target.w / 2.0;
    let cy = target.y + target.h / 2.0;
    let candidates = [
        (target.right() + gap, cy - height / 2.0),
        (target.x - gap - width, cy - height / 2.0),
        (cx - width / 2.0, target.bottom() + gap),
        (cx - width / 2.0, target.y - gap - height),
    ];
    let safe_target = Rect {
        x: target.x - 18.0 * scale,
        y: target.y - 18.0 * scale,
        w: target.w + 36.0 * scale,
        h: target.h + 36.0 * scale,
    };
    candidates
        .into_iter()
        .map(|(desired_x, desired_y)| {
            let x = desired_x.clamp(
                monitor.x + inset,
                (monitor.right() - width - inset).max(monitor.x + inset),
            );
            let y = desired_y.clamp(
                monitor.y + inset,
                (monitor.bottom() - height - inset).max(monitor.y + inset),
            );
            let rect = Rect {
                x,
                y,
                w: width,
                h: height,
            };
            let cost = overlap_area(rect, safe_target) * 1000.0
                + (x - desired_x).abs()
                + (y - desired_y).abs();
            (rect, cost)
        })
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .expect("four card placements")
        .0
}

#[derive(Clone, Copy)]
struct Arrow {
    start: (f32, f32),
    tip: (f32, f32),
    wing_a: (f32, f32),
    wing_b: (f32, f32),
}

fn make_arrow(hint: &Hint, monitor: Rect, card: Option<Rect>, scale: f32) -> Option<Arrow> {
    let anchor = hint.anchor();
    let (direction, start_distance) = if let Some(card) = card {
        let center = (card.x + card.w / 2.0, card.y + card.h / 2.0);
        let dx = center.0 - anchor.0;
        let dy = center.1 - anchor.1;
        let length = dx.hypot(dy);
        if length < 1.0 {
            return None;
        }
        let direction = (dx / length, dy / length);
        let from_center = ray_rect_distance(center, (-direction.0, -direction.1), card);
        (direction, length - from_center - 4.0 * scale)
    } else {
        // Pick the side with most available room; no card is allocated for an empty label.
        let directions = [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)];
        let direction = directions.into_iter().max_by(|a, b| {
            let room =
                |d: (f32, f32)| ray_rect_distance(anchor, d, monitor) - contour_distance(hint, d);
            room(*a).total_cmp(&room(*b))
        })?;
        let contour = contour_distance(hint, direction);
        let start = (contour + 12.0 * scale + 70.0 * scale)
            .min(ray_rect_distance(anchor, direction, monitor) - 5.0 * scale);
        (direction, start)
    };
    let tip_distance = contour_distance(hint, direction) + 12.0 * scale;
    if start_distance - tip_distance < 9.0 * scale {
        return None;
    }
    let tip = (
        anchor.0 + direction.0 * tip_distance,
        anchor.1 + direction.1 * tip_distance,
    );
    let start = (
        anchor.0 + direction.0 * start_distance,
        anchor.1 + direction.1 * start_distance,
    );
    let wing_base = (
        tip.0 + direction.0 * 7.0 * scale,
        tip.1 + direction.1 * 7.0 * scale,
    );
    let across = (-direction.1 * 4.0 * scale, direction.0 * 4.0 * scale);
    Some(Arrow {
        start,
        tip,
        wing_a: (wing_base.0 + across.0, wing_base.1 + across.1),
        wing_b: (wing_base.0 - across.0, wing_base.1 - across.1),
    })
}

fn ray_rect_distance(origin: (f32, f32), direction: (f32, f32), rect: Rect) -> f32 {
    let tx = if direction.0 > 0.00001 {
        (rect.right() - origin.0) / direction.0
    } else if direction.0 < -0.00001 {
        (rect.x - origin.0) / direction.0
    } else {
        f32::INFINITY
    };
    let ty = if direction.1 > 0.00001 {
        (rect.bottom() - origin.1) / direction.1
    } else if direction.1 < -0.00001 {
        (rect.y - origin.1) / direction.1
    } else {
        f32::INFINITY
    };
    tx.min(ty).max(0.0)
}

/// The farthest intersection matters for concave polygons: stopping at the first
/// exit could place the pointer inside a farther lobe of the same target.
fn contour_distance(hint: &Hint, direction: (f32, f32)) -> f32 {
    let anchor = hint.anchor();
    let bounds = hint.bounds();
    match hint.shape() {
        "ellipse" => {
            1.0 / ((direction.0 / (bounds.w / 2.0)).powi(2)
                + (direction.1 / (bounds.h / 2.0)).powi(2))
            .sqrt()
        }
        "polygon" => {
            let mut farthest: f32 = 0.0;
            for index in 0..hint.points.len() {
                let a = hint.points[index];
                let b = hint.points[(index + 1) % hint.points.len()];
                let edge = (b.x as f32 - a.x as f32, b.y as f32 - a.y as f32);
                let offset = (a.x as f32 - anchor.0, a.y as f32 - anchor.1);
                let cross = direction.0 * edge.1 - direction.1 * edge.0;
                if cross.abs() < 0.00001 {
                    continue;
                }
                let t = (offset.0 * edge.1 - offset.1 * edge.0) / cross;
                let u = (offset.0 * direction.1 - offset.1 * direction.0) / cross;
                if t >= 0.0 && (-0.0001..=1.0001).contains(&u) {
                    farthest = farthest.max(t);
                }
            }
            farthest
        }
        "rounded" => {
            let mut low = 0.0;
            let mut high = ray_rect_distance(anchor, direction, bounds) + 1.0;
            for _ in 0..24 {
                let middle = (low + high) / 2.0;
                if hint.contains(
                    anchor.0 + direction.0 * middle,
                    anchor.1 + direction.1 * middle,
                ) {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            high
        }
        _ => ray_rect_distance(anchor, direction, bounds),
    }
}

fn target_path(hint: &Hint) -> Result<Path, String> {
    let bounds = hint.bounds();
    let skia_rect = tiny_skia::Rect::from_xywh(bounds.x, bounds.y, bounds.w, bounds.h)
        .ok_or("Invalid target dimensions")?;
    match hint.shape() {
        "ellipse" => {
            PathBuilder::from_oval(skia_rect).ok_or_else(|| "Cannot construct ellipse".into())
        }
        "rounded" => rounded_rect(bounds, 6.0),
        "polygon" => {
            let mut builder = PathBuilder::new();
            if let Some(first) = hint.points.first() {
                builder.move_to(first.x as f32, first.y as f32);
                for point in hint.points.iter().skip(1) {
                    builder.line_to(point.x as f32, point.y as f32);
                }
                builder.close();
            }
            builder
                .finish()
                .ok_or_else(|| "Cannot construct polygon".into())
        }
        _ => Ok(PathBuilder::from_rect(skia_rect)),
    }
}

fn rounded_rect(rect: Rect, radius: f32) -> Result<Path, String> {
    let r = radius.min(rect.w / 2.0).min(rect.h / 2.0).max(0.0);
    let k = 0.552_284_8 * r;
    let mut path = PathBuilder::new();
    path.move_to(rect.x + r, rect.y);
    path.line_to(rect.right() - r, rect.y);
    path.cubic_to(
        rect.right() - r + k,
        rect.y,
        rect.right(),
        rect.y + r - k,
        rect.right(),
        rect.y + r,
    );
    path.line_to(rect.right(), rect.bottom() - r);
    path.cubic_to(
        rect.right(),
        rect.bottom() - r + k,
        rect.right() - r + k,
        rect.bottom(),
        rect.right() - r,
        rect.bottom(),
    );
    path.line_to(rect.x + r, rect.bottom());
    path.cubic_to(
        rect.x + r - k,
        rect.bottom(),
        rect.x,
        rect.bottom() - r + k,
        rect.x,
        rect.bottom() - r,
    );
    path.line_to(rect.x, rect.y + r);
    path.cubic_to(
        rect.x,
        rect.y + r - k,
        rect.x + r - k,
        rect.y,
        rect.x + r,
        rect.y,
    );
    path.close();
    path.finish()
        .ok_or_else(|| "Cannot construct rounded rectangle".into())
}

fn blend_glyph(pixmap: &mut Pixmap, x: i32, y: i32, width: usize, height: usize, coverage: &[u8]) {
    let frame_width = pixmap.width() as usize;
    let frame_height = pixmap.height() as usize;
    let data = pixmap.data_mut();
    for row in 0..height {
        let py = y + row as i32;
        if py < 0 || py as usize >= frame_height {
            continue;
        }
        for column in 0..width {
            let px = x + column as i32;
            if px < 0 || px as usize >= frame_width {
                continue;
            }
            let alpha = coverage[row * width + column] as u32;
            if alpha == 0 {
                continue;
            }
            let offset = (py as usize * frame_width + px as usize) * 4;
            let source = (245 * alpha + 127) / 255;
            for channel in 0..3 {
                data[offset + channel] = (source
                    + (data[offset + channel] as u32 * (255 - alpha) + 127) / 255)
                    .min(255) as u8;
            }
            data[offset + 3] =
                (alpha + (data[offset + 3] as u32 * (255 - alpha) + 127) / 255).min(255) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn monitor() -> Rect {
        Rect {
            x: 0.0,
            y: 0.0,
            w: 1920.0,
            h: 1080.0,
        }
    }
    fn alpha(frame: &Frame, x: i32, y: i32) -> u8 {
        let x = x - frame.x;
        let y = y - frame.y;
        if x < 0 || y < 0 || x >= frame.pixmap.width() as i32 || y >= frame.pixmap.height() as i32 {
            return 0;
        }
        frame.pixmap.data()[((y as u32 * frame.pixmap.width() + x as u32) * 4 + 3) as usize]
    }

    #[test]
    fn arrow_has_twelve_pixel_air_gap_and_target_center_is_transparent() {
        let renderer = Renderer::new().unwrap();
        for shape in ["ellipse", "rectangle", "rounded"] {
            let hint = Hint::parse(
                json!({"x":500,"y":400,"text":"","shape":shape,"width":80,"height":40}),
            )
            .unwrap();
            let frame = renderer
                .render(&hint, monitor(), 1.0, [0, 210, 255])
                .unwrap();
            let tip = frame.arrow_tip.unwrap();
            assert!(!hint.contains(tip.0, tip.1));
            let distance = (tip.0 - 500.0).hypot(tip.1 - 400.0);
            let direction = ((tip.0 - 500.0) / distance, (tip.1 - 400.0) / distance);
            assert!((distance - contour_distance(&hint, direction) - 12.0).abs() < 0.01);
            assert_eq!(alpha(&frame, 500, 400), 0);
        }
    }

    #[test]
    fn empty_text_uses_a_small_bitmap_and_no_card() {
        let renderer = Renderer::new().unwrap();
        let hint = Hint::parse(json!({"x":500,"y":400,"text":"","arrow":false})).unwrap();
        let frame = renderer
            .render(&hint, monitor(), 1.0, [250, 100, 30])
            .unwrap();
        assert!(frame.pixmap.width() <= 26 && frame.pixmap.height() <= 26);
        assert!(frame.arrow_tip.is_none());
        assert_eq!(alpha(&frame, 500, 400), 0);
    }

    #[test]
    fn polygon_and_arrow_are_independent() {
        let renderer = Renderer::new().unwrap();
        let mut hint = Hint::parse(json!({"x":500,"y":400,"text":"","shape":"polygon","arrow":false,"points":[{"x":460,"y":370},{"x":540,"y":370},{"x":540,"y":430},{"x":460,"y":430}]})).unwrap();
        let marker = renderer
            .render(&hint, monitor(), 1.0, [255, 0, 100])
            .unwrap();
        assert!(marker.arrow_tip.is_none());
        assert!(alpha(&marker, 460, 400) > 0);
        assert_eq!(alpha(&marker, 500, 400), 0);
        hint.arrow = Some(true);
        hint.target = false;
        let arrow = renderer
            .render(&hint, monitor(), 1.0, [255, 0, 100])
            .unwrap();
        assert!(arrow.arrow_tip.is_some());
        assert_eq!(alpha(&arrow, 460, 400), 0);
    }

    #[test]
    fn card_stays_inside_monitor_and_handles_cyrillic_wrapping() {
        let renderer = Renderer::new().unwrap();
        let hint = Hint::parse(json!({"x":1900,"y":1050,"text":"Нажмите настройки приложения, затем выберите нужную вкладку."})).unwrap();
        let frame = renderer
            .render(&hint, monitor(), 1.5, [0, 210, 255])
            .unwrap();
        assert!(frame.x >= 0 && frame.y >= 0);
        assert!(frame.x + frame.pixmap.width() as i32 <= 1920);
        assert!(frame.y + frame.pixmap.height() as i32 <= 1080);
        assert_eq!(alpha(&frame, 1900, 1050), 0);
        assert!(frame.arrow_tip.is_some());
    }

    #[test]
    fn concave_polygon_uses_farthest_ray_intersection() {
        let hint = Hint::parse(json!({"x":20,"y":50,"text":"","shape":"polygon","points":[{"x":0,"y":0},{"x":100,"y":0},{"x":100,"y":100},{"x":70,"y":100},{"x":70,"y":30},{"x":40,"y":30},{"x":40,"y":100},{"x":0,"y":100}]})).unwrap();
        assert!((contour_distance(&hint, (1.0, 0.0)) - 80.0).abs() < 0.01);
    }
}
