// Contrast across the local background, with color similarity as a secondary cue.
// No capture or timer here; hysteresis prevents needless color flicker.
pub const FALLBACK: [u8; 3] = [239, 35, 45];

/// Resolve colors supplied by the client before rasterizing the first frame.
/// None requests local background sampling; an explicit color always wins.
pub fn supplied_color(color: &str, palette: &[String]) -> Result<Option<[u8; 3]>, String> {
    if color != "auto" {
        return crate::model::parse_color(color).map(Some);
    }
    if palette.is_empty() {
        return Ok(None);
    }
    let pixels = palette
        .iter()
        .map(|color| crate::model::parse_color(color))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(choose(&pixels)))
}

const CANDIDATES: [[u8; 3]; 11] = [
    FALLBACK,
    [0, 229, 255],
    [230, 255, 64],
    [255, 55, 161],
    [160, 65, 255],
    [35, 80, 255],
    [255, 110, 25],
    [0, 240, 125],
    [255, 50, 60],
    [250, 250, 250],
    [16, 18, 22],
];
pub fn luminance(c: [u8; 3]) -> f32 {
    let l = |v: u8| {
        let x = v as f32 / 255.;
        if x <= 0.04045 {
            x / 12.92
        } else {
            ((x + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * l(c[0]) + 0.7152 * l(c[1]) + 0.0722 * l(c[2])
}
#[cfg(test)]
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let x = luminance(a);
    let y = luminance(b);
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
pub fn choose(pixels: &[[u8; 3]]) -> [u8; 3] {
    if pixels.is_empty() {
        return FALLBACK;
    }
    let samples = prepare_samples(pixels);
    let mut ratios = Vec::with_capacity(samples.len());
    choose_prepared(&samples, &mut ratios).0
}

#[derive(Clone, Copy)]
struct Sample {
    color: [u8; 3],
    luminance: f32,
}

#[derive(Clone, Copy)]
struct Quality {
    score: f32,
    coverage: f32,
    quartile: f32,
}

impl Quality {
    fn adequate_red(self, minimum_quartile: f32) -> bool {
        self.coverage >= 0.8 && self.quartile >= minimum_quartile
    }
}

/// One reproducible sample per contiguous stratum, with a different offset in
/// each stratum. A regular screen texture cannot alias a fixed sampling stride.
/// Luminance is calculated once, then shared by every candidate color.
fn prepare_samples(pixels: &[[u8; 3]]) -> Vec<Sample> {
    let count = pixels.len().min(1024);
    let mut samples = Vec::with_capacity(count);
    for index in 0..count {
        let start = index * pixels.len() / count;
        let end = (index + 1) * pixels.len() / count;
        let mut hash = (index as u32).wrapping_add(0x9e37_79b9);
        hash = (hash ^ (hash >> 16)).wrapping_mul(0x85eb_ca6b);
        hash = (hash ^ (hash >> 13)).wrapping_mul(0xc2b2_ae35);
        hash ^= hash >> 16;
        let color = pixels[start + hash as usize % (end - start)];
        samples.push(Sample {
            color,
            luminance: luminance(color),
        });
    }
    samples
}

fn choose_prepared(samples: &[Sample], ratios: &mut Vec<f32>) -> ([u8; 3], f32, Quality) {
    let red_quality = quality(samples, FALLBACK, ratios);
    // Preserve the primary red whenever it is legible. A numerically higher
    // score alone is not a reason to replace the intended visual identity.
    if red_quality.adequate_red(3.0) {
        return (FALLBACK, red_quality.score, red_quality);
    }
    let mut best = FALLBACK;
    let mut score_best = red_quality.score;
    for c in CANDIDATES.into_iter().skip(1) {
        let score = quality(samples, c, ratios).score;
        if score > score_best {
            score_best = score;
            best = c
        }
    }
    (best, score_best, red_quality)
}

fn quality(samples: &[Sample], c: [u8; 3], ratios: &mut Vec<f32>) -> Quality {
    if samples.is_empty() {
        return Quality {
            score: 0.0,
            coverage: 0.0,
            quartile: 0.0,
        };
    }
    ratios.clear();
    let candidate_luminance = luminance(c);
    let mut common = 0.;
    let mut covered = 0.;
    for sample in samples {
        let r = (candidate_luminance.max(sample.luminance) + 0.05)
            / (candidate_luminance.min(sample.luminance) + 0.05);
        ratios.push(r.min(10.));
        if r >= 3. {
            covered += 1.;
        }
        let distance = (0..3)
            .map(|i| (c[i] as f32 - sample.color[i] as f32).powi(2))
            .sum::<f32>();
        if distance < 10000. {
            common += 1.;
        }
    }
    let n = ratios.len() as f32;
    let quartile_index = ratios.len() / 4;
    let (_, quartile, _) = ratios.select_nth_unstable_by(quartile_index, f32::total_cmp);
    Quality {
        score: *quartile * 1.1 + covered / n * 3. - common / n * 2.
            + if c[0] != c[1] { 0.2 } else { 0. },
        coverage: covered / n,
        quartile: *quartile,
    }
}

pub fn choose_stable(pixels: &[[u8; 3]], current: [u8; 3]) -> [u8; 3] {
    if pixels.is_empty() {
        return current;
    }
    let samples = prepare_samples(pixels);
    let mut ratios = Vec::with_capacity(samples.len());
    let (candidate, candidate_score, red_quality) = choose_prepared(&samples, &mut ratios);
    if candidate == FALLBACK {
        // A small contrast margin avoids chattering exactly at 3:1, while
        // restoring red on normal light/dark surfaces even if another color
        // would have a higher raw contrast score.
        return if current == FALLBACK || red_quality.adequate_red(3.1) {
            FALLBACK
        } else {
            current
        };
    }
    let threshold = if current == FALLBACK { 0.25 } else { 0.8 };
    if candidate_score > quality(&samples, current, &mut ratios).score + threshold {
        candidate
    } else {
        current
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn supplied_colors_resolve_without_a_desktop_sample() {
        let palette = ["#EF232D".to_owned()];
        assert_eq!(
            supplied_color("#123456", &palette).unwrap(),
            Some([18, 52, 86])
        );
        let automatic = supplied_color("auto", &palette).unwrap().unwrap();
        assert_ne!(automatic, FALLBACK);
        assert!(contrast(automatic, FALLBACK) >= 3.0);
        assert_eq!(supplied_color("auto", &[]).unwrap(), None);
        assert_eq!(
            supplied_color("auto", &["#FFFFFF".to_owned()]).unwrap(),
            Some(FALLBACK)
        );
        assert!(supplied_color("#notrgb", &[]).is_err());
        assert!(supplied_color("auto", &["not a color".to_owned()]).is_err());
    }

    #[test]
    fn adapts_to_background() {
        for bg in [[255, 255, 255], [0, 0, 0], [0, 229, 255], FALLBACK] {
            let c = choose(&vec![bg; 100]);
            assert!(contrast(c, bg) >= 3.0);
            assert_ne!(c, bg)
        }
    }
    #[test]
    fn preferred_red_is_used_on_both_light_and_dark_backgrounds() {
        assert_eq!(choose(&[[255, 255, 255]; 64]), FALLBACK);
        assert_eq!(choose(&[[0, 0, 0]; 64]), FALLBACK);
        assert_eq!(choose(&[]), FALLBACK);
    }
    #[test]
    fn contrast_for_noise() {
        let c = choose(&[[0, 0, 0], [255, 255, 255]].repeat(100));
        assert!(contrast(c, [0, 0, 0]) >= 3.);
        assert!(contrast(c, [255, 255, 255]) >= 3.);
    }
    #[test]
    fn tracks_background_without_color_chatter() {
        let dark = choose(&[[0, 0, 0]; 64]);
        assert_eq!(dark, FALLBACK);
        assert_eq!(choose_stable(&[[2, 2, 2]; 64], dark), dark);
        let light = choose_stable(&[[255, 255, 255]; 64], dark);
        assert_eq!(light, FALLBACK);
        let adapted = choose_stable(&[FALLBACK; 64], light);
        assert_ne!(adapted, FALLBACK);
        assert!(contrast(adapted, FALLBACK) >= 3.0);
        assert_eq!(choose_stable(&[[255, 255, 255]; 64], adapted), FALLBACK);
        assert_eq!(choose_stable(&[[0, 0, 0]; 64], adapted), FALLBACK);
        assert_eq!(choose_stable(&[], light), light);
    }
    #[test]
    fn striped_background_does_not_alias_the_sample_stride() {
        for stripe_width in [1, 2, 4, 8, 16] {
            let pixels: Vec<_> = (0..128 * 128)
                .map(|index| {
                    if (index % 128 / stripe_width) % 2 == 0 {
                        [0, 0, 0]
                    } else {
                        [255, 255, 255]
                    }
                })
                .collect();
            let samples = prepare_samples(&pixels);
            assert_eq!(samples.len(), 1024);
            let dark = samples
                .iter()
                .filter(|sample| sample.color == [0, 0, 0])
                .count();
            assert!(
                dark > 350 && dark < 674,
                "Unbalanced sampling for stripe width {stripe_width}: {dark}"
            );
            let selected = choose(&pixels);
            assert!(contrast(selected, [0, 0, 0]) >= 3.0);
            assert!(contrast(selected, [255, 255, 255]) >= 3.0);
            assert_eq!(selected, choose(&pixels));
        }
    }
    #[test]
    fn sampling_is_bounded_and_keeps_small_palettes_intact() {
        for count in [0, 1, 16, 1023, 1024, 1025, 16_384] {
            let pixels = vec![[32, 48, 64]; count];
            assert_eq!(prepare_samples(&pixels).len(), count.min(1024));
        }
        let palette = [[10, 20, 30], [100, 120, 140], [220, 230, 240]];
        let samples = prepare_samples(&palette);
        assert_eq!(
            samples
                .iter()
                .map(|sample| sample.color)
                .collect::<Vec<_>>(),
            palette
        );
    }
}
