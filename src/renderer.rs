//! CPU rendering of small transparent bitmaps. Animation is a deterministic phase,
//! supplied by the caller; this module owns no timer and never captures the screen.

use crate::model::{Hint, Rect};
use fontdue::layout::{CoordinateSystem, Layout, LayoutSettings, TextStyle};
use fontdue::{Font, FontSettings};
use std::cell::RefCell;
use tiny_skia::{
    FillRule, LineCap, LineJoin, Paint, Path, PathBuilder, Pixmap, PixmapPaint, Stroke, Transform,
};

#[path = "sketch.rs"]
mod sketch;
use sketch::Sketch;

pub struct Frame {
    pub pixmap: Pixmap,
    pub x: i32,
    pub y: i32,
    /// Physical desktop coordinate, useful for inspection and geometry tests.
    pub arrow_tip: Option<(f32, f32)>,
    /// Fixed shaft start in physical desktop coordinates, for local contrast sampling.
    pub arrow_start: Option<(f32, f32)>,
    // Includes every arrow frame, even when an asynchronous screenshot contains
    // an earlier animation phase. Shared source pixels avoid per-frame rasters.
    arrow_sampling_mask: Option<sketch::SamplingMask>,
}

impl Frame {
    /// Move already-rendered pixels without rasterizing. Sampling coordinates
    /// and both arrow landmarks move atomically with the native window origin.
    pub fn translate(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let x = self.x.checked_add(dx).ok_or("Frame translation overflow")?;
        let y = self.y.checked_add(dy).ok_or("Frame translation overflow")?;
        if let Some(mask) = self.arrow_sampling_mask.as_mut() {
            mask.translate(dx, dy)?;
        }
        self.x = x;
        self.y = y;
        self.arrow_start = self
            .arrow_start
            .map(|(x, y)| (x + dx as f32, y + dy as f32));
        self.arrow_tip = self.arrow_tip.map(|(x, y)| (x + dx as f32, y + dy as f32));
        Ok(())
    }

    pub fn fits(&self, monitor: Rect) -> bool {
        self.x as f64 >= monitor.x as f64
            && self.y as f64 >= monitor.y as f64
            && self.x as f64 + self.pixmap.width() as f64 <= monitor.right() as f64
            && self.y as f64 + self.pixmap.height() as f64 <= monitor.bottom() as f64
    }

    /// Whether our overlay can contaminate a physical desktop sample pixel.
    pub fn occludes_sample(&self, x: i32, y: i32) -> bool {
        if self
            .arrow_sampling_mask
            .as_ref()
            .is_some_and(|mask| mask.contains(x, y))
        {
            return true;
        }
        // Keep the existing dilation for the target contour and label card.
        let px = x - self.x;
        let py = y - self.y;
        for yy in (py - 2).max(0)..=(py + 2).min(self.pixmap.height() as i32 - 1) {
            for xx in (px - 2).max(0)..=(px + 2).min(self.pixmap.width() as i32 - 1) {
                if self.pixmap.data()
                    [((yy as u32 * self.pixmap.width() + xx as u32) * 4 + 3) as usize]
                    != 0
                {
                    return true;
                }
            }
        }
        false
    }
}

pub struct Renderer {
    font: Font,
    card_cache: RefCell<Option<CachedCard>>,
    sketch_cache: RefCell<Option<Sketch>>,
}

struct CachedCard {
    text: String,
    scale: f32,
    max_width: f32,
    pixmap: Pixmap,
}

impl Renderer {
    pub fn new() -> Result<Self, String> {
        let font = Font::from_bytes(
            include_bytes!("../assets/IBMPlexMono-Regular.ttf") as &[u8],
            FontSettings::default(),
        )
        .map_err(|error| format!("Cannot load bundled font: {error}"))?;
        Ok(Self {
            font,
            card_cache: RefCell::new(None),
            sketch_cache: RefCell::new(None),
        })
    }

    pub fn render(
        &self,
        hint: &Hint,
        monitor: Rect,
        scale: f32,
        color: [u8; 3],
    ) -> Result<Frame, String> {
        self.render_at(hint, monitor, scale, color, 0.0)
    }

    pub fn render_at(
        &self,
        hint: &Hint,
        monitor: Rect,
        scale: f32,
        color: [u8; 3],
        phase: f32,
    ) -> Result<Frame, String> {
        if !scale.is_finite()
            || scale <= 0.0
            || !phase.is_finite()
            || !monitor.w.is_finite()
            || !monitor.h.is_finite()
            || monitor.w < 1.0
            || monitor.h < 1.0
        {
            return Err("Invalid display dimensions or scale".into());
        }
        let scale = scale.clamp(0.5, 4.0);
        let phase = if hint.animate { phase } else { 0.0 };
        let target = hint.bounds();
        let anchor = hint.anchor();
        let margin = 4.0 * scale;
        let padding = 9.0 * scale;
        let label = if hint.step > 1 {
            format!("{} · {}", hint.step, hint.text)
        } else {
            hint.text.clone()
        };
        let card = if hint.text.is_empty() {
            None
        } else {
            let max_width = (280.0 * scale).min(monitor.w - 2.0 * margin) - 2.0 * padding;
            if max_width < 13.0 * scale {
                return Err("Display is too narrow for the instruction".into());
            }
            let (width, height) = self.prepare_card(&label, max_width, scale)?;
            if height > monitor.h - 2.0 * margin {
                return Err(
                    "Instruction is too tall for the display; shorten it or remove line breaks"
                        .into(),
                );
            }
            Some(place_card(
                target,
                monitor,
                width,
                height,
                scale,
                if hint.has_arrow() {
                    hint.arrow_scale
                } else {
                    1.0
                },
            ))
        };

        let desired_arrow = if hint.has_arrow() {
            make_arrow(hint, monitor, card, scale)
        } else {
            None
        };
        // Marker-only and label-only hints never decode the arrow atlas.
        let mut sketches = self.sketch_cache.borrow_mut();
        let arrow = if let Some(arrow) = desired_arrow {
            if sketches.is_none() {
                *sketches = Some(Sketch::load(color)?);
            }
            let sketch = sketches.as_mut().expect("sketch loaded above");
            sketch.tint(color);
            fit_arrow(arrow, sketch, monitor, card, scale, hint.arrow_scale)
        } else {
            None
        };
        let mut bounds = if hint.target { Some(target) } else { None };
        if let Some(card) = card {
            add_bounds(&mut bounds, card);
        }
        if let Some(arrow) = arrow {
            add_bounds(
                &mut bounds,
                sketches
                    .as_ref()
                    .expect("visible arrow has a sketch")
                    .bounds(arrow.start, arrow.tip, arrow.bend_sign),
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
            let path = target_path(hint, scale, phase)?;
            let stroke = Stroke {
                width: 1.3 * scale,
                line_cap: LineCap::Round,
                line_join: LineJoin::Round,
                ..Stroke::default()
            };
            pixmap.stroke_path(&path, &paint, &stroke, transform, None);
        }
        if let Some(arrow) = arrow {
            sketches.as_ref().expect("visible arrow has a sketch").draw(
                &mut pixmap,
                arrow.start,
                arrow.tip,
                arrow.bend_sign,
                (x, y),
                phase,
            );
        }
        if let Some(card) = card {
            if let Some(cached) = self.card_cache.borrow().as_ref() {
                pixmap.draw_pixmap(
                    (card.x - x as f32).round() as i32,
                    (card.y - y as f32).round() as i32,
                    cached.pixmap.as_ref(),
                    &PixmapPaint::default(),
                    Transform::identity(),
                    None,
                );
            }
        }
        let arrow_sampling_mask = arrow
            .map(|arrow| {
                sketches
                    .as_ref()
                    .expect("visible arrow has a sketch")
                    .sampling_mask(arrow.start, arrow.tip, arrow.bend_sign)
            })
            .transpose()?;
        Ok(Frame {
            pixmap,
            x,
            y,
            arrow_tip: arrow.map(|arrow| arrow.tip),
            arrow_start: arrow.map(|arrow| arrow.start),
            arrow_sampling_mask,
        })
    }

    /// Cache one complete text card. Animation does not repeat text layout,
    /// font rasterization, or card painting; it only composites this bitmap.
    fn prepare_card(&self, text: &str, max_width: f32, scale: f32) -> Result<(f32, f32), String> {
        let mut cache = self.card_cache.borrow_mut();
        if let Some(card) = cache.as_ref() {
            if card.text == text && card.scale == scale && card.max_width == max_width {
                return Ok((card.pixmap.width() as f32, card.pixmap.height() as f32));
            }
        }
        let padding = 9.0 * scale;
        let mut layout = Layout::new(CoordinateSystem::PositiveYDown);
        layout.reset(&LayoutSettings {
            max_width: Some(max_width),
            line_height: 1.12,
            ..LayoutSettings::default()
        });
        layout.append(&[&self.font], &TextStyle::new(text, 13.0 * scale, 0));
        let text_width = layout
            .glyphs()
            .iter()
            .map(|glyph| glyph.x + glyph.width as f32)
            .fold(0.0, f32::max)
            .ceil();
        let width = (text_width.max(1.0).min(max_width) + 2.0 * padding).ceil() as u32;
        let height = (layout.height().max(13.0 * scale).ceil() + 2.0 * padding).ceil() as u32;
        if width > 8192 || height > 8192 || width as u64 * height as u64 > 8_000_000 {
            return Err("Instruction card exceeds supported dimensions".into());
        }
        let mut pixmap = Pixmap::new(width, height).ok_or("Cannot allocate text card")?;
        let path = rounded_rect(
            Rect {
                x: 0.0,
                y: 0.0,
                w: width as f32,
                h: height as f32,
            },
            7.0 * scale,
        )?;
        let mut background = Paint::default();
        background.set_color_rgba8(20, 23, 29, 231);
        background.anti_alias = true;
        pixmap.fill_path(
            &path,
            &background,
            FillRule::Winding,
            Transform::identity(),
            None,
        );
        for glyph in layout.glyphs() {
            let (metrics, coverage) = self.font.rasterize_config(glyph.key);
            blend_glyph(
                &mut pixmap,
                (padding + glyph.x).round() as i32,
                (padding + glyph.y).round() as i32,
                metrics.width,
                metrics.height,
                &coverage,
            );
        }
        *cache = Some(CachedCard {
            text: text.to_owned(),
            scale,
            max_width,
            pixmap,
        });
        Ok((width as f32, height as f32))
    }
}

fn add_bounds(bounds: &mut Option<Rect>, next: Rect) {
    *bounds = Some(bounds.map_or(next, |previous| previous.union(next)));
}

fn overlap_area(a: Rect, b: Rect) -> f32 {
    (a.right().min(b.right()) - a.x.max(b.x)).max(0.0)
        * (a.bottom().min(b.bottom()) - a.y.max(b.y)).max(0.0)
}

fn place_card(
    target: Rect,
    monitor: Rect,
    width: f32,
    height: f32,
    scale: f32,
    arrow_scale: f32,
) -> Rect {
    // Keep the target/card air gaps fixed and reserve the rounded tail's radius.
    let gap = (16.0 + 103.5 * arrow_scale) * scale;
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
    bend_sign: f32,
}

/// Keep the tip fixed and uniformly shrink the complete sprite if an edge or
/// card leaves less room. Try the opposite bend before sacrificing its size.
fn fit_arrow(
    arrow: Arrow,
    sketch: &Sketch,
    monitor: Rect,
    card: Option<Rect>,
    scale: f32,
    arrow_scale: f32,
) -> Option<Arrow> {
    let inset = 2.0 * scale;
    let safe = Rect {
        x: monitor.x + inset,
        y: monitor.y + inset,
        w: (monitor.w - 2.0 * inset).max(0.0),
        h: (monitor.h - 2.0 * inset).max(0.0),
    };
    if arrow.tip.0 < safe.x
        || arrow.tip.0 > safe.right()
        || arrow.tip.1 < safe.y
        || arrow.tip.1 > safe.bottom()
    {
        return None;
    }
    let length = (arrow.start.0 - arrow.tip.0).hypot(arrow.start.1 - arrow.tip.1);
    let mut best = None;
    let mut best_factor = 0.0;
    for bend_sign in [arrow.bend_sign, -arrow.bend_sign] {
        let bounds = sketch.bounds(arrow.start, arrow.tip, bend_sign);
        let mut factor = 1.0_f32;
        for (edge, limit, origin) in [
            (bounds.x, safe.x, arrow.tip.0),
            (bounds.right(), safe.right(), arrow.tip.0),
            (bounds.y, safe.y, arrow.tip.1),
            (bounds.bottom(), safe.bottom(), arrow.tip.1),
        ] {
            if (edge < limit && limit <= origin) || (edge > limit && limit >= origin) {
                factor = factor.min(((limit - origin) / (edge - origin)).clamp(0.0, 1.0));
            }
        }
        if let Some(card) = card {
            let padding = 2.0 * scale;
            let avoid = Rect {
                x: card.x - padding,
                y: card.y - padding,
                w: card.w + 2.0 * padding,
                h: card.h + 2.0 * padding,
            };
            let scaled_bounds = |f: f32| Rect {
                x: arrow.tip.0 + (bounds.x - arrow.tip.0) * f,
                y: arrow.tip.1 + (bounds.y - arrow.tip.1) * f,
                w: bounds.w * f,
                h: bounds.h * f,
            };
            if overlap_area(scaled_bounds(factor), avoid) > 0.0 {
                let mut low = 0.0;
                let mut high = factor;
                for _ in 0..16 {
                    let middle = (low + high) / 2.0;
                    if overlap_area(scaled_bounds(middle), avoid) > 0.0 {
                        high = middle;
                    } else {
                        low = middle;
                    }
                }
                factor = low;
            }
        }
        if factor > best_factor && length * factor >= 14.0 * scale * arrow_scale {
            best_factor = factor;
            best = Some(Arrow {
                start: (
                    arrow.tip.0 + (arrow.start.0 - arrow.tip.0) * factor,
                    arrow.tip.1 + (arrow.start.1 - arrow.tip.1) * factor,
                ),
                tip: arrow.tip,
                bend_sign,
            });
        }
        if factor >= 1.0 {
            break;
        }
    }
    best
}

fn make_arrow(hint: &Hint, monitor: Rect, card: Option<Rect>, scale: f32) -> Option<Arrow> {
    let anchor = hint.anchor();
    let geometry_scale = scale * hint.arrow_scale;
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
        (
            direction,
            length - from_center - 4.0 * scale - 3.5 * geometry_scale,
        )
    } else {
        // Pick the side with most available room; no card is allocated for an empty label.
        let directions = [(-1.0, 0.0), (1.0, 0.0), (0.0, -1.0), (0.0, 1.0)];
        let direction = directions.into_iter().max_by(|a, b| {
            let room =
                |d: (f32, f32)| ray_rect_distance(anchor, d, monitor) - contour_distance(hint, d);
            room(*a).total_cmp(&room(*b))
        })?;
        let contour = contour_distance(hint, direction);
        let start = (contour + 12.0 * scale + 100.0 * geometry_scale).min(
            ray_rect_distance(anchor, direction, monitor) - 4.0 * scale - 3.5 * geometry_scale,
        );
        (direction, start)
    };
    let tip_distance = contour_distance(hint, direction) + 12.0 * scale;
    if start_distance - tip_distance < 14.0 * geometry_scale {
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
    let midpoint = ((start.0 + tip.0) * 0.5, (start.1 + tip.1) * 0.5);
    let normal = (direction.1, -direction.0);
    let bend_sign = if ray_rect_distance(midpoint, normal, monitor)
        >= ray_rect_distance(midpoint, (-normal.0, -normal.1), monitor)
    {
        1.0
    } else {
        -1.0
    };
    Some(Arrow {
        start,
        tip,
        bend_sign,
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

fn target_path(hint: &Hint, scale: f32, phase: f32) -> Result<Path, String> {
    let bounds = hint.bounds();
    let amplitude = (1.1 * scale).min(bounds.w.min(bounds.h) * 0.08);
    let mut builder = PathBuilder::new();
    match hint.shape() {
        "ellipse" => {
            let cx = bounds.x + bounds.w / 2.0;
            let cy = bounds.y + bounds.h / 2.0;
            let rx = bounds.w / 2.0;
            let ry = bounds.h / 2.0;
            let delta = std::f32::consts::TAU / 16.0;
            let k = (delta / 4.0).tan() * 4.0 / 3.0;
            let point = |angle: f32| {
                let wave = amplitude
                    * (0.65 * (angle * 3.0 + phase * 0.65).sin()
                        + 0.30 * (angle * 5.0 - phase * 0.39 + 0.8).sin());
                (
                    cx + (rx + wave) * angle.cos(),
                    cy + (ry + wave) * angle.sin(),
                )
            };
            let first = point(0.0);
            builder.move_to(first.0, first.1);
            for index in 0..16 {
                let a = index as f32 * delta;
                let b = (index + 1) as f32 * delta;
                let start = point(a);
                let end = if index == 15 { first } else { point(b) };
                builder.cubic_to(
                    start.0 - k * rx * a.sin(),
                    start.1 + k * ry * a.cos(),
                    end.0 + k * rx * b.sin(),
                    end.1 - k * ry * b.cos(),
                    end.0,
                    end.1,
                );
            }
        }
        "rounded" => {
            let r = 6.0_f32.min(bounds.w / 2.0).min(bounds.h / 2.0);
            let k = 0.552_284_8 * r;
            let top_left = (bounds.x + r, bounds.y);
            let top_right = (bounds.right() - r, bounds.y);
            let right_top = (bounds.right(), bounds.y + r);
            let right_bottom = (bounds.right(), bounds.bottom() - r);
            let bottom_right = (bounds.right() - r, bounds.bottom());
            let bottom_left = (bounds.x + r, bounds.bottom());
            let left_bottom = (bounds.x, bounds.bottom() - r);
            let left_top = (bounds.x, bounds.y + r);
            builder.move_to(top_left.0, top_left.1);
            ink_edge(&mut builder, top_left, top_right, amplitude, phase, 0);
            builder.cubic_to(
                top_right.0 + k,
                top_right.1,
                right_top.0,
                right_top.1 - k,
                right_top.0,
                right_top.1,
            );
            ink_edge(&mut builder, right_top, right_bottom, amplitude, phase, 1);
            builder.cubic_to(
                right_bottom.0,
                right_bottom.1 + k,
                bottom_right.0 + k,
                bottom_right.1,
                bottom_right.0,
                bottom_right.1,
            );
            ink_edge(&mut builder, bottom_right, bottom_left, amplitude, phase, 2);
            builder.cubic_to(
                bottom_left.0 - k,
                bottom_left.1,
                left_bottom.0,
                left_bottom.1 + k,
                left_bottom.0,
                left_bottom.1,
            );
            ink_edge(&mut builder, left_bottom, left_top, amplitude, phase, 3);
            builder.cubic_to(
                left_top.0,
                left_top.1 - k,
                top_left.0 - k,
                top_left.1,
                top_left.0,
                top_left.1,
            );
        }
        "polygon" => {
            if let Some(first) = hint.points.first() {
                builder.move_to(first.x as f32, first.y as f32);
                for index in 0..hint.points.len() {
                    let a = hint.points[index];
                    let b = hint.points[(index + 1) % hint.points.len()];
                    ink_edge(
                        &mut builder,
                        (a.x as f32, a.y as f32),
                        (b.x as f32, b.y as f32),
                        amplitude,
                        phase,
                        index,
                    );
                }
            }
        }
        _ => {
            let points = [
                (bounds.x, bounds.y),
                (bounds.right(), bounds.y),
                (bounds.right(), bounds.bottom()),
                (bounds.x, bounds.bottom()),
            ];
            builder.move_to(points[0].0, points[0].1);
            for index in 0..points.len() {
                ink_edge(
                    &mut builder,
                    points[index],
                    points[(index + 1) % points.len()],
                    amplitude,
                    phase,
                    index,
                );
            }
        }
    }
    builder.close();
    builder
        .finish()
        .ok_or_else(|| "Cannot construct target outline".into())
}

/// Anchored vertices preserve the chosen shape while each edge gains a tiny,
/// slowly changing bend. The convex combination stays within the amplitude.
fn ink_edge(
    builder: &mut PathBuilder,
    start: (f32, f32),
    end: (f32, f32),
    amplitude: f32,
    phase: f32,
    index: usize,
) {
    let dx = end.0 - start.0;
    let dy = end.1 - start.1;
    let length = dx.hypot(dy).max(0.001);
    let normal = (-dy / length, dx / length);
    let seed = index as f32 * 1.73;
    let a =
        amplitude * (0.7 * (phase * 0.65 + seed + 0.6).sin() + 0.25 * (phase * 0.41 - seed).sin());
    let b = amplitude
        * (0.65 * (phase * 0.59 + seed + 1.9).sin() + 0.3 * (phase * 0.37 - seed + 0.4).sin());
    builder.cubic_to(
        start.0 + dx / 3.0 + normal.0 * a,
        start.1 + dy / 3.0 + normal.1 * a,
        start.0 + dx * 2.0 / 3.0 + normal.0 * b,
        start.1 + dy * 2.0 / 3.0 + normal.1 * b,
        end.0,
        end.1,
    );
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

    #[test]
    fn changing_phase_changes_ink_without_moving_tip_or_window() {
        let renderer = Renderer::new().unwrap();
        for shape in ["ellipse", "rectangle", "rounded"] {
            let hint = Hint::parse(
                json!({"x":500,"y":400,"text":"","shape":shape,"width":80,"height":40}),
            )
            .unwrap();
            let first = renderer
                .render_at(&hint, monitor(), 1.0, [10, 210, 255], 0.0)
                .unwrap();
            let second = renderer
                .render_at(&hint, monitor(), 1.0, [10, 210, 255], 1.0)
                .unwrap();
            assert_eq!(first.arrow_tip, second.arrow_tip);
            assert_eq!(first.arrow_start, second.arrow_start);
            assert_eq!(
                (
                    first.x,
                    first.y,
                    first.pixmap.width(),
                    first.pixmap.height()
                ),
                (
                    second.x,
                    second.y,
                    second.pixmap.width(),
                    second.pixmap.height()
                )
            );
            assert_ne!(first.pixmap.data(), second.pixmap.data());
            assert_eq!(alpha(&first, 500, 400), 0);
            assert_eq!(alpha(&second, 500, 400), 0);
        }
    }

    #[test]
    fn animation_reuses_card_pixels_and_font_rasterization() {
        let renderer = Renderer::new().unwrap();
        let hint =
            Hint::parse(json!({"x":500,"y":400,"text":"Нажмите настройки","step":2})).unwrap();
        let first = renderer
            .render_at(&hint, monitor(), 1.0, [10, 210, 255], 0.0)
            .unwrap();
        let (address, pixels, width, height) = {
            let cache = renderer.card_cache.borrow();
            let card = cache.as_ref().unwrap();
            (
                card.pixmap.data().as_ptr(),
                card.pixmap.data().to_vec(),
                card.pixmap.width(),
                card.pixmap.height(),
            )
        };
        let second = renderer
            .render_at(&hint, monitor(), 1.0, [10, 210, 255], 1.0)
            .unwrap();
        {
            let cache = renderer.card_cache.borrow();
            assert_eq!(address, cache.as_ref().unwrap().pixmap.data().as_ptr());
            assert_eq!(pixels, cache.as_ref().unwrap().pixmap.data());
        }
        let card = place_card(
            hint.bounds(),
            monitor(),
            width as f32,
            height as f32,
            1.0,
            hint.arrow_scale,
        );
        assert_eq!(first.arrow_tip, second.arrow_tip);
        for y in card.y.ceil() as i32..card.bottom().floor() as i32 {
            for x in card.x.ceil() as i32..card.right().floor() as i32 {
                let pixel = |frame: &Frame| {
                    let offset = (((y - frame.y) as u32 * frame.pixmap.width()
                        + (x - frame.x) as u32)
                        * 4) as usize;
                    let data = frame.pixmap.data();
                    [
                        data[offset],
                        data[offset + 1],
                        data[offset + 2],
                        data[offset + 3],
                    ]
                };
                assert_eq!(pixel(&first), pixel(&second));
            }
        }
    }

    #[test]
    fn four_sketch_frames_preserve_the_gap_in_all_four_orientations() {
        let renderer = Renderer::new().unwrap();
        for (x, y) in [(50, 540), (1870, 540), (960, 50), (960, 1030)] {
            let hint = Hint::parse(json!({"x":x,"y":y,"text":"","target":false})).unwrap();
            let mut previous: Option<Frame> = None;
            for phase in [0.01, 0.21, 0.41, 0.61] {
                let frame = renderer
                    .render_at(&hint, monitor(), 1.0, [239, 35, 45], phase)
                    .unwrap();
                let tip = frame.arrow_tip.unwrap();
                let distance = (tip.0 - x as f32).hypot(tip.1 - y as f32);
                let direction = ((tip.0 - x as f32) / distance, (tip.1 - y as f32) / distance);
                assert!((distance - contour_distance(&hint, direction) - 12.0).abs() < 0.001);
                for offset in 0..12 {
                    assert_eq!(
                        alpha(
                            &frame,
                            (tip.0 - direction.0 * offset as f32).round() as i32,
                            (tip.1 - direction.1 * offset as f32).round() as i32
                        ),
                        0
                    );
                }
                assert_eq!(alpha(&frame, x, y), 0);
                assert!(frame
                    .pixmap
                    .data()
                    .chunks_exact(4)
                    .any(|pixel| pixel[3] > 200));
                if let Some(previous) = &previous {
                    assert_eq!(frame.arrow_start, previous.arrow_start);
                    assert_eq!(frame.arrow_tip, previous.arrow_tip);
                    assert_eq!(
                        (
                            frame.x,
                            frame.y,
                            frame.pixmap.width(),
                            frame.pixmap.height()
                        ),
                        (
                            previous.x,
                            previous.y,
                            previous.pixmap.width(),
                            previous.pixmap.height()
                        )
                    );
                    assert_ne!(frame.pixmap.data(), previous.pixmap.data());
                }
                previous = Some(frame);
            }
        }
    }

    #[test]
    fn larger_arrow_keeps_target_gap_and_scales_length_and_head() {
        let base = Hint::parse(json!({"x":500,"y":400,"text":"","arrow_scale":1.0})).unwrap();
        let larger = Hint::parse(json!({"x":500,"y":400,"text":""})).unwrap();
        let a = make_arrow(&base, monitor(), None, 1.0).unwrap();
        let b = make_arrow(&larger, monitor(), None, 1.0).unwrap();
        assert_eq!(a.tip, b.tip);
        let shaft_length =
            |arrow: Arrow| (arrow.start.0 - arrow.tip.0).hypot(arrow.start.1 - arrow.tip.1);
        assert!((shaft_length(b) / shaft_length(a) - 1.6).abs() < 0.001);
        assert!((shaft_length(b) - 160.0).abs() < 0.001);
        let sketch = Sketch::load([239, 35, 45]).unwrap();
        let small_bounds = sketch.bounds(a.start, a.tip, a.bend_sign);
        let large_bounds = sketch.bounds(b.start, b.tip, b.bend_sign);
        assert!((large_bounds.w / small_bounds.w - 1.6).abs() < 0.001);
        assert!((large_bounds.h / small_bounds.h - 1.6).abs() < 0.001);
        assert_eq!(base.bounds().w, larger.bounds().w);
        assert_eq!(base.bounds().h, larger.bounds().h);
        let renderer = Renderer::new().unwrap();
        let frame = renderer
            .render_at(&larger, monitor(), 1.0, [0, 210, 255], 1.0)
            .unwrap();
        assert_eq!(frame.arrow_start, Some(b.start));
        assert_eq!(alpha(&frame, 500, 400), 0);
    }

    #[test]
    fn text_hint_leaves_room_for_larger_arrow() {
        let base = Hint::parse(json!({"x":500,"y":400,"arrow_scale":1.0})).unwrap();
        let larger = Hint::parse(json!({"x":500,"y":400})).unwrap();
        let card_a = place_card(base.bounds(), monitor(), 160.0, 40.0, 1.0, base.arrow_scale);
        let card_b = place_card(
            larger.bounds(),
            monitor(),
            160.0,
            40.0,
            1.0,
            larger.arrow_scale,
        );
        let a = make_arrow(&base, monitor(), Some(card_a), 1.0).unwrap();
        let b = make_arrow(&larger, monitor(), Some(card_b), 1.0).unwrap();
        let length =
            |arrow: Arrow| (arrow.start.0 - arrow.tip.0).hypot(arrow.start.1 - arrow.tip.1);
        assert!((length(b) / length(a) - 1.6).abs() < 0.001);
        assert!((length(b) - 160.0).abs() < 0.001);
        assert_eq!(a.tip, b.tip);
    }

    #[test]
    fn sketch_cache_is_lazy_and_reused_and_disabled_animation_stays_still() {
        let renderer = Renderer::new().unwrap();
        assert!(renderer.sketch_cache.borrow().is_none());
        let mut hint =
            Hint::parse(json!({"x":500,"y":400,"text":"","arrow":false,"animate":false})).unwrap();
        renderer
            .render(&hint, monitor(), 1.0, [239, 35, 45])
            .unwrap();
        assert!(renderer.sketch_cache.borrow().is_none());
        hint.arrow = Some(true);
        let first = renderer
            .render_at(&hint, monitor(), 1.0, [239, 35, 45], 0.0)
            .unwrap();
        let addresses = renderer
            .sketch_cache
            .borrow()
            .as_ref()
            .unwrap()
            .frame_addresses();
        let second = renderer
            .render_at(&hint, monitor(), 1.0, [239, 35, 45], 0.6)
            .unwrap();
        assert_eq!(first.pixmap.data(), second.pixmap.data());
        renderer
            .render_at(&hint, monitor(), 1.0, [20, 220, 130], 0.6)
            .unwrap();
        assert_eq!(
            addresses,
            renderer
                .sketch_cache
                .borrow()
                .as_ref()
                .unwrap()
                .frame_addresses()
        );
    }

    #[test]
    fn fitting_the_full_sketch_keeps_tip_fixed_and_avoids_edges_and_card() {
        let sketch = Sketch::load([239, 35, 45]).unwrap();
        let screen = Rect {
            x: 0.0,
            y: 0.0,
            w: 240.0,
            h: 160.0,
        };
        let desired = Arrow {
            start: (-40.0, 100.0),
            tip: (200.0, 100.0),
            bend_sign: 1.0,
        };
        for card in [
            None,
            Some(Rect {
                x: 10.0,
                y: 80.0,
                w: 100.0,
                h: 40.0,
            }),
        ] {
            let fitted = fit_arrow(desired, &sketch, screen, card, 1.0, 1.6).unwrap();
            assert_eq!(desired.tip, fitted.tip);
            assert!(fitted.start.0 > desired.start.0);
            let bounds = sketch.bounds(fitted.start, fitted.tip, fitted.bend_sign);
            assert!(bounds.x >= 1.99 && bounds.y >= 1.99);
            assert!(bounds.right() <= 238.01 && bounds.bottom() <= 158.01);
            if let Some(card) = card {
                assert_eq!(overlap_area(bounds, card), 0.0);
            }
        }
    }

    #[test]
    fn asynchronous_samples_mask_every_arrow_phase_with_shared_storage() {
        let renderer = Renderer::new().unwrap();
        let hint = Hint::parse(json!({"x":700,"y":500,"text":"","target":false,
            "arrow_scale":2.5}))
        .unwrap();
        let current = renderer
            .render_at(&hint, monitor(), 2.0, [239, 35, 45], 0.01)
            .unwrap();
        let mut missed_by_old_mask = 0;
        for phase in [0.21, 0.41, 0.61] {
            let earlier = renderer
                .render_at(&hint, monitor(), 2.0, [239, 35, 45], phase)
                .unwrap();
            assert!(current
                .arrow_sampling_mask
                .as_ref()
                .unwrap()
                .shares_storage(earlier.arrow_sampling_mask.as_ref().unwrap()));
            for y in 0..earlier.pixmap.height() as i32 {
                for x in 0..earlier.pixmap.width() as i32 {
                    let px = earlier.x + x;
                    let py = earlier.y + y;
                    if alpha(&earlier, px, py) == 0 {
                        continue;
                    }
                    assert!(
                        current.occludes_sample(px, py),
                        "unmasked old-frame ink at {px},{py}"
                    );
                    let old_mask =
                        (-2..=2).any(|dy| (-2..=2).any(|dx| alpha(&current, px + dx, py + dy) > 0));
                    if !old_mask {
                        missed_by_old_mask += 1;
                    }
                }
            }
        }
        assert!(
            missed_by_old_mask > 20,
            "test must include motion beyond the previous 2px mask"
        );
        assert!(!current.occludes_sample(700, 500));
        assert!(!current.occludes_sample(10, 10));
    }

    #[test]
    fn sampling_mask_keeps_card_and_target_without_loading_an_arrow() {
        let renderer = Renderer::new().unwrap();
        let hint = Hint::parse(json!({"x":500,"y":400,"text":"Settings","arrow":false})).unwrap();
        let frame = renderer
            .render(&hint, monitor(), 1.0, [239, 35, 45])
            .unwrap();
        assert!(frame.arrow_sampling_mask.is_none());
        assert!(renderer.sketch_cache.borrow().is_none());
        let mut occupied = 0;
        for y in 0..frame.pixmap.height() as i32 {
            for x in 0..frame.pixmap.width() as i32 {
                let px = frame.x + x;
                let py = frame.y + y;
                if alpha(&frame, px, py) != 0 {
                    occupied += 1;
                    assert!(frame.occludes_sample(px, py));
                }
            }
        }
        assert!(occupied > 1000);
        assert!(!frame.occludes_sample(500, 400));
    }

    #[test]
    fn translating_a_frame_moves_sampling_and_landmarks_without_repainting() {
        let renderer = Renderer::new().unwrap();
        let hint = Hint::parse(json!({"x":500,"y":400,"text":"Settings"})).unwrap();
        let mut frame = renderer
            .render_at(&hint, monitor(), 1.0, [239, 35, 45], 0.41)
            .unwrap();
        let original_origin = (frame.x, frame.y);
        let original_start = frame.arrow_start.unwrap();
        let original_tip = frame.arrow_tip.unwrap();
        let pointer = frame.pixmap.data().as_ptr();
        let pixels = frame.pixmap.data().to_vec();
        let mut probes = Vec::new();
        for y in (-4..frame.pixmap.height() as i32 + 4).step_by(3) {
            for x in (-4..frame.pixmap.width() as i32 + 4).step_by(3) {
                let px = frame.x + x;
                let py = frame.y + y;
                probes.push((px, py, frame.occludes_sample(px, py)));
            }
        }
        frame.translate(37, -14).unwrap();
        assert_eq!(
            (frame.x, frame.y),
            (original_origin.0 + 37, original_origin.1 - 14)
        );
        assert_eq!(
            frame.arrow_start,
            Some((original_start.0 + 37.0, original_start.1 - 14.0))
        );
        assert_eq!(
            frame.arrow_tip,
            Some((original_tip.0 + 37.0, original_tip.1 - 14.0))
        );
        assert_eq!(pointer, frame.pixmap.data().as_ptr());
        assert_eq!(pixels, frame.pixmap.data());
        assert!(frame.fits(monitor()));
        for &(x, y, occupied) in &probes {
            assert_eq!(occupied, frame.occludes_sample(x + 37, y - 14));
        }
        let current_origin = (frame.x, frame.y);
        assert!(frame.translate(i32::MAX, 0).is_err());
        assert_eq!(current_origin, (frame.x, frame.y));
        for &(x, y, occupied) in &probes {
            assert_eq!(occupied, frame.occludes_sample(x + 37, y - 14));
        }
        frame.translate(1920, 0).unwrap();
        assert!(!frame.fits(monitor()));
    }
}
