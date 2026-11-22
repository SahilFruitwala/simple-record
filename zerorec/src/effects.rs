//! Picture filters a layer can have: color correction and a chroma key
//! (green screen). The key's math is here, as a color lookup table the
//! compositor hands to CoreImage (`CIColorCubeWithColorSpace`), so it runs
//! on the GPU and can be tested without one.
//!
//! The key works like OBS's: a pixel's distance from the key color in the
//! chroma (CbCr) plane, so brightness doesn't matter; past `similarity` it
//! fades in over `smoothness`, and pixels close to the key lose saturation
//! (`spill`), so green light reflected on a face goes gray, not green.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChromaKey {
    pub color: [u8; 3],
    /// Chroma distance, 0 to 1, keyed out completely.
    pub similarity: f64,
    /// The distance over which pixels go from keyed out to kept.
    pub smoothness: f64,
    /// The distance over which pixels near the key lose their color.
    pub spill: f64,
}

impl Default for ChromaKey {
    fn default() -> Self {
        ChromaKey { color: [0, 255, 0], similarity: 0.4, smoothness: 0.08, spill: 0.1 }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    /// Added to every channel, -1 to 1.
    pub brightness: f64,
    /// 0 to 2 (1: unchanged).
    pub contrast: f64,
    /// 0 (gray) to 2 (1: unchanged).
    pub saturation: f64,
    pub chroma_key: Option<ChromaKey>,
}

impl Default for Effects {
    fn default() -> Self {
        Effects { brightness: 0.0, contrast: 1.0, saturation: 1.0, chroma_key: None }
    }
}

impl Effects {
    /// Whether the color controls change anything.
    pub fn has_color(&self) -> bool {
        self.brightness != 0.0 || self.contrast != 1.0 || self.saturation != 1.0
    }

    pub fn is_none(&self) -> bool {
        !self.has_color() && self.chroma_key.is_none()
    }
}

/// Rec. 709 chroma of an RGB color (0..1 each), centered on 0.5.
fn cbcr([r, g, b]: [f64; 3]) -> [f64; 2] {
    [-0.1146 * r - 0.3854 * g + 0.5 * b + 0.5, 0.5 * r - 0.4542 * g - 0.0458 * b + 0.5]
}

fn saturate(v: f64) -> f64 {
    v.clamp(0.0, 1.0)
}

/// The keyed color of `rgb` (0..1): straight (not premultiplied) RGB and
/// alpha.
pub fn chroma_key(k: &ChromaKey, rgb: [f64; 3]) -> ([f64; 3], f64) {
    let key = cbcr(k.color.map(|c| c as f64 / 255.0));
    let c = cbcr(rgb);
    let dist = ((c[0] - key[0]).powi(2) + (c[1] - key[1]).powi(2)).sqrt();
    let base = dist - k.similarity;
    let alpha = saturate(base / k.smoothness.max(1e-4)).powf(1.5);
    let keep = saturate(base / k.spill.max(1e-4)).powf(1.5);
    let gray = 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2];
    (rgb.map(|v| gray + (v - gray) * keep), alpha)
}

/// The key as an `n`x`n`x`n` color cube for CoreImage: premultiplied RGBA
/// floats, red varying fastest, then green, then blue.
pub fn chroma_cube(k: &ChromaKey, n: usize) -> Vec<f32> {
    let n = n.max(2);
    let step = 1.0 / (n - 1) as f64;
    let mut out = Vec::with_capacity(n * n * n * 4);
    for b in 0..n {
        for g in 0..n {
            for r in 0..n {
                let (c, a) = chroma_key(k, [r as f64 * step, g as f64 * step, b as f64 * step]);
                out.extend(c.map(|v| (v * a) as f32));
                out.push(a as f32);
            }
        }
    }
    out
}

/// The cube size used: fine enough for smooth key edges (32 KB x 16).
pub const CUBE_SIZE: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn green_goes_and_skin_stays() {
        let k = ChromaKey::default();
        assert_eq!(chroma_key(&k, [0.0, 1.0, 0.0]).1, 0.0);
        // Darker and lighter greens too: brightness doesn't matter.
        assert_eq!(chroma_key(&k, [0.1, 0.5, 0.1]).1, 0.0);
        assert!(chroma_key(&k, [0.2, 0.7, 0.25]).1 < 0.05);
        for kept in [[0.9, 0.7, 0.6], [0.5, 0.5, 0.5], [0.2, 0.2, 0.8], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]] {
            assert_eq!(chroma_key(&k, kept).1, 1.0, "{kept:?}");
        }
        // A blue screen.
        let blue = ChromaKey { color: [0, 0, 255], ..k };
        assert_eq!(chroma_key(&blue, [0.05, 0.1, 0.9]).1, 0.0);
        assert_eq!(chroma_key(&blue, [0.1, 0.9, 0.1]).1, 1.0);
    }

    #[test]
    fn edges_are_soft_and_spill_goes_gray() {
        let k = ChromaKey::default();
        // Along a line from green to gray, alpha only rises.
        let mut last = 0.0;
        let mut partial = 0;
        for i in 0..=100 {
            let t = i as f64 / 100.0;
            let a = chroma_key(&k, [0.5 * t, 1.0 - 0.5 * t, 0.5 * t]).1;
            assert!(a >= last);
            partial += (a > 0.0 && a < 1.0) as usize;
            last = a;
        }
        assert!(partial >= 3, "a soft edge: {partial} steps");
        // A greenish pixel that's kept loses most of its green cast.
        let ([r, g, b], a) = chroma_key(&k, [0.35, 0.65, 0.35]);
        assert!(a > 0.0 && g - r < 0.15 && (r - b).abs() < 1e-9, "{r} {g} {b} {a}");
        // Far from the key, nothing changes.
        let (c, _) = chroma_key(&k, [0.9, 0.2, 0.3]);
        assert!(c.iter().zip([0.9, 0.2, 0.3]).all(|(a, b)| (a - b).abs() < 1e-9), "{c:?}");
    }

    #[test]
    fn cube_layout() {
        let k = ChromaKey::default();
        let n = 4;
        let c = chroma_cube(&k, n);
        assert_eq!(c.len(), n * n * n * 4);
        let at = |r: usize, g: usize, b: usize| &c[((b * n + g) * n + r) * 4..][..4];
        assert_eq!(at(0, n - 1, 0), &[0.0, 0.0, 0.0, 0.0], "pure green: transparent");
        assert_eq!(at(n - 1, 0, 0), &[1.0, 0.0, 0.0, 1.0], "red: kept");
        // Premultiplied: no channel above alpha.
        assert!(c.chunks(4).all(|p| p[..3].iter().all(|v| *v <= p[3] + 1e-6)));
    }

    #[test]
    fn neutral_effects() {
        assert!(Effects::default().is_none());
        assert!(Effects { saturation: 0.0, ..Default::default() }.has_color());
        let json = r#"{"chroma_key":{"color":[0,0,255]}}"#;
        let e: Effects = serde_json::from_str(json).unwrap();
        assert_eq!(e.chroma_key.unwrap().similarity, 0.4);
        assert!(!e.has_color());
    }
}
