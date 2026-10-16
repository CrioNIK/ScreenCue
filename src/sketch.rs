//! The selected hand-drawn arrow, decoded once and recolored only on demand.
use crate::model::Rect;
use std::cell::RefCell;
use std::rc::Rc;
use tiny_skia::{FilterQuality, Pixmap, PixmapPaint, Transform};

const WIDTH: u32 = 256;
const HEIGHT: u32 = 174;
const ATLAS: &[u8] = include_bytes!("../assets/sketch-atlas.png");
const TAIL: (f32, f32) = (24.0, 156.0);
const TIP: (f32, f32) = (235.52, 15.352_941);
const MAX_PLACED_BYTES: u64 = 2 * 1024 * 1024;

pub(super) struct Sketch {
    frames: Vec<Pixmap>,
    // The union of occupied rows, including the bilinear filter footprint.
    // This keeps bounds tight under rotation and identical for every frame.
    envelope: Vec<(f32, f32)>,
    sampling_union: Rc<[u8]>,
    placed: RefCell<Option<PlacedFrames>>,
    color: Option<[u8; 3]>,
}

#[derive(Clone, Copy, PartialEq)]
struct PlacementKey {
    start: (f32, f32),
    tip: (f32, f32),
    bend_sign: f32,
}

struct PlacedFrames {
    key: PlacementKey,
    origin: (i32, i32),
    frames: Vec<Pixmap>,
}

impl PlacedFrames {
    fn translate_to(&mut self, key: PlacementKey) -> bool {
        let dx = key.tip.0 - self.key.tip.0;
        let dy = key.tip.1 - self.key.tip.1;
        if key.bend_sign != self.key.bend_sign
            || !dx.is_finite()
            || !dy.is_finite()
            || dx as f64 > i32::MAX as f64
            || (dx as f64) < i32::MIN as f64
            || dy as f64 > i32::MAX as f64
            || (dy as f64) < i32::MIN as f64
            || dx != dx.round()
            || dy != dy.round()
            || (key.start.0 - self.key.start.0 - dx).abs() > 0.0001
            || (key.start.1 - self.key.start.1 - dy).abs() > 0.0001
        {
            return false;
        }
        let Some(x) = self.origin.0.checked_add(dx as i32) else {
            return false;
        };
        let Some(y) = self.origin.1.checked_add(dy as i32) else {
            return false;
        };
        self.key = key;
        self.origin = (x, y);
        true
    }
}

/// One immutable mask for every animation frame, shared by successive renders.
/// Only its small inverse transform changes when hint geometry changes.
pub(super) struct SamplingMask {
    pixels: Rc<[u8]>,
    inverse: Transform,
    translation: (i32, i32),
}

impl SamplingMask {
    pub(super) fn contains(&self, x: i32, y: i32) -> bool {
        let source = map(
            self.inverse,
            (
                (x as i64 - self.translation.0 as i64) as f32 + 0.5,
                (y as i64 - self.translation.1 as i64) as f32 + 0.5,
            ),
        );
        if source.0 < -1.0
            || source.1 < -1.0
            || source.0 > WIDTH as f32 + 1.0
            || source.1 > HEIGHT as f32 + 1.0
        {
            return false;
        }
        let sx = source.0.floor() as i32;
        let sy = source.1.floor() as i32;
        // Include every texel that bilinear filtering can contribute. The extra
        // neighbor is conservative at rotated/subpixel boundaries.
        for yy in (sy - 1).max(0)..=(sy + 1).min(HEIGHT as i32 - 1) {
            for xx in (sx - 1).max(0)..=(sx + 1).min(WIDTH as i32 - 1) {
                if self.pixels[(yy as u32 * WIDTH + xx as u32) as usize] != 0 {
                    return true;
                }
            }
        }
        false
    }

    pub(super) fn translate(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let x = self
            .translation
            .0
            .checked_add(dx)
            .ok_or("Sampling translation overflow")?;
        let y = self
            .translation
            .1
            .checked_add(dy)
            .ok_or("Sampling translation overflow")?;
        self.translation = (x, y);
        Ok(())
    }

    #[cfg(test)]
    pub(super) fn shares_storage(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.pixels, &other.pixels)
    }
}

impl Sketch {
    pub(super) fn load(color: [u8; 3]) -> Result<Self, String> {
        let atlas = Pixmap::decode_png(ATLAS)
            .map_err(|error| format!("Cannot decode bundled sketch arrow: {error}"))?;
        if (atlas.width(), atlas.height()) != (WIDTH * 2, HEIGHT * 2) {
            return Err("Bundled sketch atlas has unexpected dimensions".into());
        }
        let axis = source_axis();
        // Remove only the few generated flecks beyond the landmark. Keep a
        // filter-sized guard so rotated/interpolated ink cannot cross the tip.
        let guard = 1.5 * (axis.0.abs() + axis.1.abs());
        let mut rows = vec![None::<(u32, u32)>; HEIGHT as usize];
        let mut sampling_union = vec![0u8; (WIDTH * HEIGHT) as usize];
        let mut frames = Vec::with_capacity(4);
        for index in 0..4 {
            let mut frame = Pixmap::new(WIDTH, HEIGHT).ok_or("Cannot allocate sketch frame")?;
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    let source_x = x + index % 2 * WIDTH;
                    let source_y = y + index / 2 * HEIGHT;
                    let source = ((source_y * atlas.width() + source_x) * 4) as usize;
                    let destination = ((y * WIDTH + x) * 4) as usize;
                    let projection =
                        (x as f32 + 0.5 - TIP.0) * axis.0 + (y as f32 + 0.5 - TIP.1) * axis.1;
                    let alpha = if projection <= -guard {
                        atlas.data()[source + 3]
                    } else {
                        0
                    };
                    frame.data_mut()[destination + 3] = alpha;
                    if alpha != 0 {
                        sampling_union[destination / 4] = 1;
                        rows[y as usize] = Some(
                            rows[y as usize]
                                .map_or((x, x), |(left, right)| (left.min(x), right.max(x))),
                        );
                    }
                }
            }
            frames.push(frame);
        }
        let mut envelope = Vec::with_capacity(HEIGHT as usize * 4);
        for (y, row) in rows.into_iter().enumerate() {
            if let Some((left, right)) = row {
                let left = left as f32 - 1.0;
                let right = right as f32 + 2.0;
                let top = y as f32 - 1.0;
                let bottom = y as f32 + 2.0;
                envelope.extend([(left, top), (right, top), (left, bottom), (right, bottom)]);
            }
        }
        if envelope.is_empty() {
            return Err("Bundled sketch atlas contains no visible ink".into());
        }
        let mut sketch = Self {
            frames,
            envelope,
            sampling_union: sampling_union.into(),
            placed: RefCell::new(None),
            color: None,
        };
        sketch.tint(color);
        Ok(sketch)
    }

    pub(super) fn tint(&mut self, color: [u8; 3]) {
        if self.color == Some(color) {
            return;
        }
        // Only one hint's transformed frames are retained, never a history of
        // positions/colors. The source alpha and sampling mask stay unchanged.
        self.placed.get_mut().take();
        for frame in &mut self.frames {
            for pixel in frame.data_mut().chunks_exact_mut(4) {
                let alpha = pixel[3] as u32;
                for channel in 0..3 {
                    pixel[channel] = ((color[channel] as u32 * alpha + 127) / 255) as u8;
                }
            }
        }
        self.color = Some(color);
    }

    pub(super) fn bounds(&self, start: (f32, f32), tip: (f32, f32), bend_sign: f32) -> Rect {
        let transform = placement(start, tip, bend_sign);
        let mut left = f32::INFINITY;
        let mut top = f32::INFINITY;
        let mut right = f32::NEG_INFINITY;
        let mut bottom = f32::NEG_INFINITY;
        for &(x, y) in &self.envelope {
            let (x, y) = map(transform, (x, y));
            left = left.min(x);
            top = top.min(y);
            right = right.max(x);
            bottom = bottom.max(y);
        }
        Rect {
            x: left,
            y: top,
            w: right - left,
            h: bottom - top,
        }
    }

    pub(super) fn draw(
        &self,
        destination: &mut Pixmap,
        start: (f32, f32),
        tip: (f32, f32),
        bend_sign: f32,
        origin: (i32, i32),
        phase: f32,
    ) {
        let key = PlacementKey {
            start,
            tip,
            bend_sign,
        };
        let mut placed = self.placed.borrow_mut();
        if let Some(cached) = placed.as_mut() {
            if cached.key != key && !cached.translate_to(key) {
                *placed = None;
            }
        }
        if placed.is_none() {
            *placed = self.prepare_placed(key);
        }
        if let Some(cached) = placed.as_ref() {
            destination.draw_pixmap(
                cached.origin.0 - origin.0,
                cached.origin.1 - origin.1,
                cached.frames[frame_index(phase)].as_ref(),
                &PixmapPaint::default(),
                Transform::identity(),
                None,
            );
            return;
        }
        // Large high-DPI hints remain bounded: resample directly instead of
        // retaining four buffers above the explicit cache size limit.
        self.draw_direct(destination, key, origin, frame_index(phase));
    }

    fn prepare_placed(&self, key: PlacementKey) -> Option<PlacedFrames> {
        let bounds = self.bounds(key.start, key.tip, key.bend_sign);
        let origin = (bounds.x.floor() as i32, bounds.y.floor() as i32);
        let width = (bounds.right().ceil() - origin.0 as f32).max(1.0) as u32;
        let height = (bounds.bottom().ceil() - origin.1 as f32).max(1.0) as u32;
        if width as u64 * height as u64 * 4 * 4 > MAX_PLACED_BYTES {
            return None;
        }
        let mut frames = Vec::with_capacity(4);
        for index in 0..4 {
            let mut frame = Pixmap::new(width, height)?;
            self.draw_direct(&mut frame, key, origin, index);
            frames.push(frame);
        }
        Some(PlacedFrames {
            key,
            origin,
            frames,
        })
    }

    fn draw_direct(
        &self,
        destination: &mut Pixmap,
        key: PlacementKey,
        origin: (i32, i32),
        index: usize,
    ) {
        let PlacementKey {
            start,
            tip,
            bend_sign,
        } = key;
        let mut transform = placement(start, tip, bend_sign);
        transform.tx -= origin.0 as f32;
        transform.ty -= origin.1 as f32;
        let paint = PixmapPaint {
            quality: FilterQuality::Bilinear,
            ..PixmapPaint::default()
        };
        destination.draw_pixmap(0, 0, self.frames[index].as_ref(), &paint, transform, None);
    }

    pub(super) fn sampling_mask(
        &self,
        start: (f32, f32),
        tip: (f32, f32),
        bend_sign: f32,
    ) -> Result<SamplingMask, String> {
        Ok(SamplingMask {
            pixels: Rc::clone(&self.sampling_union),
            inverse: placement(start, tip, bend_sign)
                .invert()
                .ok_or("Cannot invert sketch sampling transform")?,
            translation: (0, 0),
        })
    }

    #[cfg(test)]
    pub(super) fn frame_addresses(&self) -> Vec<usize> {
        self.frames
            .iter()
            .map(|frame| frame.data().as_ptr() as usize)
            .collect()
    }
}

fn source_axis() -> (f32, f32) {
    let length = (TIP.0 - TAIL.0).hypot(TIP.1 - TAIL.1);
    ((TIP.0 - TAIL.0) / length, (TIP.1 - TAIL.1) / length)
}

fn placement(start: (f32, f32), tip: (f32, f32), bend_sign: f32) -> Transform {
    let source = source_axis();
    let source_normal = (-source.1, source.0);
    let source_length = (TIP.0 - TAIL.0).hypot(TIP.1 - TAIL.1);
    let along = (
        (tip.0 - start.0) / source_length,
        (tip.1 - start.1) / source_length,
    );
    let across = (-along.1 * bend_sign, along.0 * bend_sign);
    let a = along.0 * source.0 + across.0 * source_normal.0;
    let b = along.1 * source.0 + across.1 * source_normal.0;
    let c = along.0 * source.1 + across.0 * source_normal.1;
    let d = along.1 * source.1 + across.1 * source_normal.1;
    Transform::from_row(
        a,
        b,
        c,
        d,
        tip.0 - a * TIP.0 - c * TIP.1,
        tip.1 - b * TIP.0 - d * TIP.1,
    )
}

fn map(transform: Transform, point: (f32, f32)) -> (f32, f32) {
    (
        transform.sx * point.0 + transform.kx * point.1 + transform.tx,
        transform.ky * point.0 + transform.sy * point.1 + transform.ty,
    )
}

fn frame_index(phase: f32) -> usize {
    [0, 1, 2, 3, 2, 1][((phase.max(0.0) * 5.0).floor() as u64 % 6) as usize]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_frames_are_small_distinct_and_loop_at_five_steps_per_second() {
        assert!(ATLAS.len() <= 128 * 1024);
        let sketch = Sketch::load([239, 35, 45]).unwrap();
        assert_eq!(sketch.frames.len(), 4);
        for frame in &sketch.frames {
            assert_eq!((frame.width(), frame.height()), (WIDTH, HEIGHT));
            assert!(
                frame
                    .data()
                    .chunks_exact(4)
                    .filter(|pixel| pixel[3] != 0)
                    .count()
                    > 5000
            );
        }
        for pair in sketch.frames.windows(2) {
            assert_ne!(pair[0].data(), pair[1].data());
        }
        let sequence: Vec<_> = (0..7).map(|i| frame_index(i as f32 * 0.2 + 0.01)).collect();
        assert_eq!(sequence, [0, 1, 2, 3, 2, 1, 0]);
    }

    #[test]
    fn recoloring_reuses_frames_and_preserves_premultiplied_alpha_texture() {
        let mut sketch = Sketch::load([239, 35, 45]).unwrap();
        let addresses = sketch.frame_addresses();
        let alpha: Vec<_> = sketch
            .frames
            .iter()
            .flat_map(|f| f.data().chunks_exact(4).map(|p| p[3]))
            .collect();
        let color = [16, 220, 127];
        sketch.tint(color);
        sketch.tint(color);
        assert_eq!(addresses, sketch.frame_addresses());
        let recolored_alpha: Vec<_> = sketch
            .frames
            .iter()
            .flat_map(|f| f.data().chunks_exact(4).map(|p| p[3]))
            .collect();
        assert_eq!(alpha, recolored_alpha);
        for frame in &sketch.frames {
            for pixel in frame.data().chunks_exact(4) {
                for channel in 0..3 {
                    assert_eq!(
                        pixel[channel],
                        ((color[channel] as u32 * pixel[3] as u32 + 127) / 255) as u8
                    );
                }
            }
        }
    }

    #[test]
    fn transforms_fix_both_landmarks_in_every_orientation_and_reflection() {
        for direction in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0), (0.6, 0.8)] {
            for sign in [-1.0, 1.0] {
                let tip = (400.0, 300.0);
                let start = (tip.0 - direction.0 * 160.0, tip.1 - direction.1 * 160.0);
                let transform = placement(start, tip, sign);
                let mapped_tail = map(transform, TAIL);
                let mapped_tip = map(transform, TIP);
                assert!((mapped_tail.0 - start.0).hypot(mapped_tail.1 - start.1) < 0.001);
                assert!((mapped_tip.0 - tip.0).hypot(mapped_tip.1 - tip.1) < 0.001);
            }
        }
    }

    #[test]
    fn all_frames_keep_ink_behind_the_fixed_tip() {
        let sketch = Sketch::load([239, 35, 45]).unwrap();
        let axis = source_axis();
        for frame in &sketch.frames {
            for (index, pixel) in frame.data().chunks_exact(4).enumerate() {
                if pixel[3] != 0 {
                    let projection = (index as u32 % WIDTH) as f32 + 0.5 - TIP.0;
                    let vertical = (index as u32 / WIDTH) as f32 + 0.5 - TIP.1;
                    assert!(projection * axis.0 + vertical * axis.1 < -1.0);
                }
            }
        }
    }

    #[test]
    fn transformed_frame_cache_is_reused_invalidated_and_bounded() {
        let mut sketch = Sketch::load([239, 35, 45]).unwrap();
        let mut destination = Pixmap::new(600, 400).unwrap();
        let start = (60.0, 280.0);
        let tip = (400.0, 100.0);
        sketch.draw(&mut destination, start, tip, 1.0, (0, 0), 0.01);
        let addresses: Vec<_> = sketch
            .placed
            .borrow()
            .as_ref()
            .unwrap()
            .frames
            .iter()
            .map(|frame| frame.data().as_ptr() as usize)
            .collect();
        for phase in [0.21, 0.41, 0.61] {
            sketch.draw(&mut destination, start, tip, 1.0, (0, 0), phase);
            let placed = sketch.placed.borrow();
            let placed = placed.as_ref().unwrap();
            assert_eq!(
                addresses,
                placed
                    .frames
                    .iter()
                    .map(|f| f.data().as_ptr() as usize)
                    .collect::<Vec<_>>()
            );
            assert!(
                placed
                    .frames
                    .iter()
                    .map(|f| f.data().len() as u64)
                    .sum::<u64>()
                    <= MAX_PLACED_BYTES
            );
        }
        let original_origin = sketch.placed.borrow().as_ref().unwrap().origin;
        sketch.draw(
            &mut destination,
            (start.0 + 37.0, start.1 - 14.0),
            (tip.0 + 37.0, tip.1 - 14.0),
            1.0,
            (0, 0),
            0.21,
        );
        {
            let placed = sketch.placed.borrow();
            let placed = placed.as_ref().unwrap();
            assert_eq!(
                placed.origin,
                (original_origin.0 + 37, original_origin.1 - 14)
            );
            assert_eq!(
                addresses,
                placed
                    .frames
                    .iter()
                    .map(|f| f.data().as_ptr() as usize)
                    .collect::<Vec<_>>()
            );
        }
        sketch.tint([20, 220, 130]);
        assert!(sketch.placed.borrow().is_none());
        sketch.draw(&mut destination, start, tip, 1.0, (0, 0), 0.01);
        assert!(sketch.placed.borrow().is_some());
        let too_large = PlacementKey {
            start: (0.0, 0.0),
            tip: (5000.0, 0.0),
            bend_sign: 1.0,
        };
        assert!(sketch.prepare_placed(too_large).is_none());
        // Use a comfortably smaller placement here: reflecting the original
        // long diagonal can legitimately exceed the 2 MiB cache budget.
        let changed_tip = (260.0, 180.0);
        sketch.draw(&mut destination, start, changed_tip, -1.0, (0, 0), 0.21);
        assert_eq!(
            sketch.placed.borrow().as_ref().unwrap().key.tip,
            changed_tip
        );
    }
}
