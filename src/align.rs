//! Alignement d'un visage sur le gabarit 112x112 de SFace (5 points).
//!
//! Même gabarit et même principe que `cv::FaceRecognizerSF::alignCrop` :
//! similarité (rotation + échelle + translation) au sens des moindres carrés,
//! puis interpolation bilinéaire avec bord noir.

use image::{Rgb, RgbImage};

pub const ALIGNED_SIZE: u32 = 112;
pub const SFACE_TEMPLATE: [[f32; 2]; 5] =
    [[38.2946, 51.6963], [73.5318, 51.5014], [56.0252, 71.7366], [41.5493, 92.3655], [70.7299, 92.2041]];

/// Matrice 2x3 `[a, -b, tx, b, a, ty]` qui envoie `src` sur `dst` au mieux
/// (solution fermée d'Umeyama restreinte au plan, sans réflexion).
pub fn similarity(src: &[[f32; 2]; 5], dst: &[[f32; 2]; 5]) -> [f64; 6] {
    let n = src.len() as f64;
    let mean = |p: &[[f32; 2]; 5]| {
        let (sx, sy) = p.iter().fold((0.0, 0.0), |(x, y), q| (x + q[0] as f64, y + q[1] as f64));
        (sx / n, sy / n)
    };
    let (msx, msy) = mean(src);
    let (mdx, mdy) = mean(dst);
    let (mut num_a, mut num_b, mut den) = (0.0, 0.0, 0.0);
    for (s, d) in src.iter().zip(dst) {
        let (sx, sy) = (s[0] as f64 - msx, s[1] as f64 - msy);
        let (dx, dy) = (d[0] as f64 - mdx, d[1] as f64 - mdy);
        num_a += sx * dx + sy * dy;
        num_b += sx * dy - sy * dx;
        den += sx * sx + sy * sy;
    }
    if den == 0.0 {
        return [1.0, 0.0, mdx - msx, 0.0, 1.0, mdy - msy];
    }
    let a = num_a / den;
    let b = num_b / den;
    let tx = mdx - (a * msx - b * msy);
    let ty = mdy - (b * msx + a * msy);
    [a, -b, tx, b, a, ty]
}

/// Applique la transformation directe `m` (source -> destination) par
/// échantillonnage inverse bilinéaire ; hors image : noir.
pub fn warp_affine(img: &RgbImage, m: [f64; 6], out_w: u32, out_h: u32) -> RgbImage {
    let [a, _, tx, b, _, ty] = m;
    let det = a * a + b * b;
    let mut out = RgbImage::new(out_w, out_h);
    if det == 0.0 {
        return out;
    }
    // Inverse de [[a, -b], [b, a]] : [[a, b], [-b, a]] / det
    let (iw, ih) = (img.width() as i64, img.height() as i64);
    let fetch = |x: i64, y: i64| -> [f64; 3] {
        if x < 0 || y < 0 || x >= iw || y >= ih {
            [0.0; 3]
        } else {
            let p = img.get_pixel(x as u32, y as u32);
            [p[0] as f64, p[1] as f64, p[2] as f64]
        }
    };
    for v in 0..out_h {
        for u in 0..out_w {
            let du = u as f64 - tx;
            let dv = v as f64 - ty;
            let sx = (a * du + b * dv) / det;
            let sy = (-b * du + a * dv) / det;
            let x0 = sx.floor();
            let y0 = sy.floor();
            let fx = sx - x0;
            let fy = sy - y0;
            let (x0, y0) = (x0 as i64, y0 as i64);
            let p00 = fetch(x0, y0);
            let p10 = fetch(x0 + 1, y0);
            let p01 = fetch(x0, y0 + 1);
            let p11 = fetch(x0 + 1, y0 + 1);
            let mut px = [0u8; 3];
            for c in 0..3 {
                let top = p00[c] * (1.0 - fx) + p10[c] * fx;
                let bot = p01[c] * (1.0 - fx) + p11[c] * fx;
                px[c] = (top * (1.0 - fy) + bot * fy).round().clamp(0.0, 255.0) as u8;
            }
            out.put_pixel(u, v, Rgb(px));
        }
    }
    out
}

pub fn align_face(img: &RgbImage, landmarks: &[[f32; 2]; 5]) -> RgbImage {
    let m = similarity(landmarks, &SFACE_TEMPLATE);
    warp_affine(img, m, ALIGNED_SIZE, ALIGNED_SIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn apply(m: &[f64; 6], p: [f32; 2]) -> [f64; 2] {
        let (x, y) = (p[0] as f64, p[1] as f64);
        [m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]]
    }

    #[test]
    fn recovers_known_similarity() {
        // src = template tourné de 30°, mis à l'échelle 2,5 et translaté
        let (s, t) = (2.5f64, 30f64.to_radians());
        let src = SFACE_TEMPLATE.map(|[x, y]| {
            let (x, y) = (x as f64, y as f64);
            [(s * (t.cos() * x - t.sin() * y) + 200.0) as f32, (s * (t.sin() * x + t.cos() * y) + 80.0) as f32]
        });
        let m = similarity(&src, &SFACE_TEMPLATE);
        for (p, q) in src.iter().zip(SFACE_TEMPLATE) {
            let r = apply(&m, *p);
            assert!((r[0] - q[0] as f64).abs() < 0.5 && (r[1] - q[1] as f64).abs() < 0.5, "{r:?} vs {q:?}");
        }
    }

    #[test]
    fn identity_warp_copies_pixels() {
        let mut img = RgbImage::new(4, 4);
        img.put_pixel(2, 1, Rgb([200, 100, 50]));
        let out = warp_affine(&img, [1.0, 0.0, 0.0, 0.0, 1.0, 0.0], 4, 4);
        assert_eq!(out.get_pixel(2, 1), &Rgb([200, 100, 50]));
        assert_eq!(out.get_pixel(0, 0), &Rgb([0, 0, 0]));
    }
}
