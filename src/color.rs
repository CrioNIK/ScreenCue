// One-shot perceptual contrast + local color rarity. No capture or timer here.
pub const FALLBACK: [u8; 3] = [0, 210, 255];
const CANDIDATES: [[u8; 3]; 10] = [
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
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f32 {
    let x = luminance(a);
    let y = luminance(b);
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}
pub fn choose(pixels: &[[u8; 3]]) -> [u8; 3] {
    if pixels.is_empty() {
        return FALLBACK;
    }
    let mut best = FALLBACK;
    let mut score_best = f32::NEG_INFINITY;
    for c in CANDIDATES {
        let mut ratios = Vec::new();
        let mut common = 0.;
        let mut covered = 0.;
        for p in pixels.iter().step_by((pixels.len() / 1024).max(1)) {
            let r = contrast(c, *p);
            ratios.push(r.min(10.));
            if r >= 3. {
                covered += 1.
            }
            let distance = (0..3)
                .map(|i| (c[i] as f32 - p[i] as f32).powi(2))
                .sum::<f32>()
                .sqrt();
            if distance < 100. {
                common += 1.
            }
        }
        ratios.sort_by(f32::total_cmp);
        let n = ratios.len() as f32;
        let score = ratios[ratios.len() / 4] * 1.1 + covered / n * 3. - common / n * 4.
            + if c[0] != c[1] { 0.2 } else { 0. };
        if score > score_best {
            score_best = score;
            best = c
        }
    }
    best
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn adapts_to_background() {
        for bg in [[255, 255, 255], [0, 0, 0], [0, 229, 255]] {
            let c = choose(&vec![bg; 100]);
            assert!(contrast(c, bg) >= 4.5);
            assert_ne!(c, bg)
        }
    }
    #[test]
    fn contrast_for_noise() {
        let c = choose(&[[0, 0, 0], [255, 255, 255]].repeat(100));
        assert!(contrast(c, [0, 0, 0]) >= 3.);
        assert!(contrast(c, [255, 255, 255]) >= 3.);
    }
}
