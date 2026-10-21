//! Perceptual color math for palette extraction.
//!
//! Oklab (Björn Ottosson, 2020) replaces raw RGB distance and HSV saturation:
//! equal Oklab steps are close to equal perceived steps, so two colors that look
//! different never collapse into one swatch, and a vivid color is not outranked
//! by a near-black one that merely has HSV saturation 1.0.

/// sRGB 8-bit channel -> linear light.
fn lin(c: u8) -> f64 {
    let c = c as f64 / 255.0;
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// 8-bit sRGB -> Oklab `(L, a, b)`. L is 0..1, a/b are roughly -0.4..0.4.
pub fn srgb_to_oklab(r: u8, g: u8, b: u8) -> (f64, f64, f64) {
    let (r, g, b) = (lin(r), lin(g), lin(b));
    let l = 0.412_221_470_8 * r + 0.536_332_536_3 * g + 0.051_445_992_9 * b;
    let m = 0.211_903_498_2 * r + 0.680_699_545_1 * g + 0.107_396_956_6 * b;
    let s = 0.088_302_461_9 * r + 0.281_718_837_6 * g + 0.629_978_700_5 * b;
    let (l_, m_, s_) = (l.cbrt(), m.cbrt(), s.cbrt());
    (
        0.210_454_255_3 * l_ + 0.793_617_785_0 * m_ - 0.004_072_046_8 * s_,
        1.977_998_495_1 * l_ - 2.428_592_205_0 * m_ + 0.450_593_709_9 * s_,
        0.025_904_037_1 * l_ + 0.782_771_766_2 * m_ - 0.808_675_766_0 * s_,
    )
}

/// Euclidean distance in Oklab. Approximate perceptual difference, not CIEDE2000.
pub fn delta_e(a: (f64, f64, f64), b: (f64, f64, f64)) -> f64 {
    let (dl, da, db) = (a.0 - b.0, a.1 - b.1, a.2 - b.2);
    (dl * dl + da * da + db * db).sqrt()
}

/// Oklab -> OKLCH `(L, C, h_deg)`; hue is 0..360 and is meaningless at C = 0.
pub fn oklch(lab: (f64, f64, f64)) -> (f64, f64, f64) {
    let c = (lab.1 * lab.1 + lab.2 * lab.2).sqrt();
    let mut h = lab.2.atan2(lab.1).to_degrees();
    if h < 0.0 {
        h += 360.0;
    }
    (lab.0, c, h)
}

/// Swatches closer than this are the same color and merge.
pub const MERGE_DE: f64 = 0.03;
/// Swatches at least this far apart are visibly different and both stay.
pub const DISTINCT_DE: f64 = 0.06;
/// Chroma that counts as fully vivid for scoring (sRGB vivid colors sit near 0.15-0.25).
pub const VIVID_CHROMA: f64 = 0.20;

/// How much a color should win a palette slot. 0.2 floor keeps neutral colors usable.
pub fn salience(lab: (f64, f64, f64)) -> f64 {
    let (l, c, _h) = oklch(lab);
    let chroma_weight = (c / VIVID_CHROMA).clamp(0.0, 1.0);
    // Peak at L 0.62; dark and near-white colors still count, but rank below midtones.
    let lightness_weight = (1.0 - ((l - 0.62).abs() / 0.62)).clamp(0.10, 1.0);
    (0.2 + 0.8 * chroma_weight * lightness_weight).clamp(0.05, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn white_is_light_and_neutral() {
        let lab = srgb_to_oklab(255, 255, 255);
        assert!((lab.0 - 1.0).abs() < 1e-3, "L={}", lab.0);
        let (_l, c, _h) = oklch(lab);
        assert!(c < 1e-4, "C={c}");
    }

    #[test]
    fn black_is_zero() {
        let lab = srgb_to_oklab(0, 0, 0);
        assert!(lab.0.abs() < 1e-6 && lab.1.abs() < 1e-6 && lab.2.abs() < 1e-6);
    }

    #[test]
    fn brand_crimson_is_red_hued() {
        let (_l, _c, h) = oklch(srgb_to_oklab(0xc8, 0x00, 0x44));
        assert!((h - 22.0).abs() < 10.0, "h={h}"); // Oklch places this red near 20 deg
    }

    #[test]
    fn perceptually_equal_steps_differ_from_rgb_steps() {
        // Two RGB pairs with the same channel sum differ in perception.
        let a = srgb_to_oklab(0, 0, 0);
        let b = delta_e(a, srgb_to_oklab(30, 30, 30));
        let c = delta_e(a, srgb_to_oklab(90, 0, 0));
        assert!(b > 0.0 && c > 0.0);
        assert!((b - c).abs() > 1e-3, "dark gray and dark red stop being twins");
    }

    #[test]
    fn saturated_dark_loses_to_midtone() {
        let maroon = salience(srgb_to_oklab(0x4a, 0x00, 0x10));
        let coral = salience(srgb_to_oklab(0xd9, 0x6a, 0x5a));
        assert!(coral > maroon, "maroon={maroon} coral={coral}");
    }

    #[test]
    fn gray_still_scores_above_floor() {
        let gray = salience(srgb_to_oklab(0x80, 0x80, 0x80));
        assert!(gray >= 0.2, "gray={gray}");
    }
}
