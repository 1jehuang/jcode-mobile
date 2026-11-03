//! Vendored verbatim from jcode-website crates/site-visual/src/donut.rs (keep in sync).
#![allow(dead_code)]
fn cos(x: f64) -> f64 { x.cos() }
fn sin(x: f64) -> f64 { x.sin() }

// Historical donut.c lighting, not an approximation to be replaced: using
// FRAC_1_SQRT_2 would change the shared brand's calibrated field and SVG bytes.
#[allow(clippy::approx_constant)]
const LIGHT_Y: f64 = 0.7071;

#[derive(Clone, Copy, Debug)]
pub struct DonutOptions {
    pub columns: usize,
    pub rows: usize,
    pub tilt: f64,
    pub spin: f64,
    pub steps: u32,
    pub refine: bool,
    pub sharp: bool,
}

pub fn render(options: DonutOptions, output: &mut [f32]) {
    let DonutOptions {
        columns,
        rows,
        tilt,
        spin,
        steps,
        refine,
        sharp,
    } = options;
    assert_eq!(output.len(), columns * rows);
    let k1 = (columns as f64 * 5.0 * 3.0) / (8.0 * 3.0);
    let bound = 3.0 + 0.02;
    let cc = 25.0 - bound * bound;
    let (ca, sa, cb, sb) = (cos(tilt), sin(tilt), cos(spin), sin(spin));
    let (m00, m01) = (cb, sb);
    let (m10, m11, m12) = (sa * sb, -sa * cb, ca);
    let (m20, m21, m22) = (-ca * sb, ca * cb, sa);
    let (oy, oz) = (-ca * 5.0, -sa * 5.0);
    output.fill(0.0);
    for yc in 0..rows {
        let pyw = -(yc as f64 + 0.5 - rows as f64 / 2.0) / k1;
        for xc in 0..columns {
            let pxw = (xc as f64 + 0.5 - columns as f64 / 2.0) / k1;
            let inv = 1.0 / (pxw * pxw + pyw * pyw + 1.0).sqrt();
            let (wx, wy, wz) = (pxw * inv, pyw * inv, inv);
            let dx = m00 * wx + m01 * wy;
            let dy = m10 * wx + m11 * wy + m12 * wz;
            let dz = m20 * wx + m21 * wy + m22 * wz;
            let distance = |s: f64| {
                let (px, py, pz) = (dx * s, oy + dy * s, oz + dz * s);
                let q = (px * px + py * py).sqrt() - 2.0;
                (q * q + pz * pz).sqrt() - 1.0
            };
            let b = oy * dy + oz * dz;
            let disc = b * b - cc;
            if disc <= 0.0 {
                continue;
            }
            let sq = disc.sqrt();
            let exit = -b + sq;
            if exit <= 0.0 {
                continue;
            }
            let mut s = (-b - sq).max(0.0);
            let (mut minimum, mut nearest, mut hit) = (1e9, s, false);
            let (mut previous, mut after, mut last) = (s, -1.0, s);
            for _ in 0..steps {
                let d = distance(s);
                if d < minimum {
                    minimum = d;
                    previous = last;
                    nearest = s;
                    after = -1.0;
                } else if after < 0.0 {
                    after = s;
                }
                if d < 5e-4 {
                    hit = true;
                    break;
                }
                last = s;
                s += d;
                if s > exit {
                    break;
                }
            }
            if sharp && !hit {
                continue;
            }
            if refine && !hit && minimum < 0.5 {
                let mut lo = previous;
                let mut hi = if after > 0.0 {
                    after
                } else {
                    nearest + (nearest - previous)
                };
                for _ in 0..12 {
                    let m1 = lo + (hi - lo) / 3.0;
                    let m2 = hi - (hi - lo) / 3.0;
                    let (d1, d2) = (distance(m1), distance(m2));
                    if d1 < d2 {
                        hi = m2;
                        if d1 < minimum {
                            minimum = d1;
                            nearest = m1;
                        }
                    } else {
                        lo = m1;
                        if d2 < minimum {
                            minimum = d2;
                            nearest = m2;
                        }
                    }
                }
            }
            let sp = if hit { s } else { nearest };
            let aa = (1.5 * sp) / k1;
            if !hit && minimum >= aa {
                continue;
            }
            let (px, py, pz) = (dx * sp, oy + dy * sp, oz + dz * sp);
            let k = 2.0 / (px * px + py * py).sqrt();
            let (mut nx, mut ny, mut nz) = (px * (1.0 - k), py * (1.0 - k), pz);
            let length = (nx * nx + ny * ny + nz * nz).sqrt();
            let length = if length == 0.0 { 1.0 } else { length };
            nx /= length;
            ny /= length;
            nz /= length;
            let nwy = m01 * nx + m11 * ny + m21 * nz;
            let nwz = m12 * ny + m22 * nz;
            let mut value = nwy * LIGHT_Y + nwz * -LIGHT_Y;
            if value <= 0.0 {
                continue;
            }
            if !hit {
                value *= 1.0 - minimum / aa;
            }
            output[xc + yc * columns] = value as f32;
        }
    }
}

/// Separable 1-2-1 Gaussian with f32 intermediate stores, like Float32Array.
pub fn smooth(input: &[f32], columns: usize, rows: usize) -> Vec<f32> {
    assert_eq!(input.len(), columns * rows);
    let mut temporary = vec![0.0f32; input.len()];
    let mut output = vec![0.0f32; input.len()];
    for y in 0..rows {
        for x in 0..columns {
            let (mut sum, mut weight) = (0.0, 0.0);
            for offset in -1isize..=1 {
                let xx = x as isize + offset;
                if xx < 0 || xx >= columns as isize {
                    continue;
                }
                let w = if offset == 0 { 2.0 } else { 1.0 };
                sum += input[xx as usize + y * columns] as f64 * w;
                weight += w;
            }
            temporary[x + y * columns] = (sum / weight) as f32;
        }
    }
    for y in 0..rows {
        for x in 0..columns {
            let (mut sum, mut weight) = (0.0, 0.0);
            for offset in -1isize..=1 {
                let yy = y as isize + offset;
                if yy < 0 || yy >= rows as isize {
                    continue;
                }
                let w = if offset == 0 { 2.0 } else { 1.0 };
                sum += temporary[x + yy as usize * columns] as f64 * w;
                weight += w;
            }
            output[x + y * columns] = (sum / weight) as f32;
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn profiles_are_deterministic_finite_and_lit() {
        for steps in [48, 64, 96] {
            for sharp in [false, true] {
                let options = DonutOptions {
                    columns: 32,
                    rows: 24,
                    tilt: 1.15,
                    spin: 2.16,
                    steps,
                    refine: steps == 48,
                    sharp,
                };
                let mut first = vec![f32::NAN; 32 * 24];
                let mut second = vec![0.0; first.len()];
                render(options, &mut first);
                render(options, &mut second);
                assert_eq!(first, second);
                assert!(
                    first
                        .iter()
                        .all(|v| v.is_finite() && *v >= 0.0 && *v <= 1.0)
                );
                assert!(first.iter().filter(|v| **v > 0.0).count() > 50);
            }
        }
    }
    #[test]
    fn smoothing_preserves_constant_and_one_pixel_fields() {
        assert_eq!(smooth(&[0.5; 9], 3, 3), vec![0.5; 9]);
        assert_eq!(smooth(&[0.7], 1, 1), vec![0.7]);
    }
}
