//! [SPEC.md](../../SPEC.md) §4 transform-attribute parser (v0 subset).
//!
//! Scoped to `<surface>`'s child `<path>` elements at this milestone; the
//! global transform pipeline (composing `transform` onto every shape per
//! SPEC §4.3) is still a roadmap item, but its parser can reuse this
//! module unchanged when it lands.
//!
//! Supported functions in v0: `translate`, `translate3d`, `translateZ`,
//! `rotate`, `rotateX`, `rotateY`, `rotateZ`. Any other function name
//! (or any malformed token) causes the parse to fail; per SPEC §2.4 the
//! whole attribute is then in error and the caller MUST ignore it.

use std::f32::consts::PI;

/// A 4×4 transform matrix stored in column-major order (matching the
/// `matrix3d()` convention in SPEC §4.2.9 and CSS-TRANSFORMS-2 §11).
///
/// `m.0[col][row]` is the `(row, col)`-th element of the matrix.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Mat4(pub [[f32; 4]; 4]);

impl Mat4 {
    pub(crate) fn identity() -> Self {
        let mut m = [[0.0_f32; 4]; 4];
        m[0][0] = 1.0;
        m[1][1] = 1.0;
        m[2][2] = 1.0;
        m[3][3] = 1.0;
        Self(m)
    }

    fn translation(tx: f32, ty: f32, tz: f32) -> Self {
        let mut m = Self::identity();
        m.0[3] = [tx, ty, tz, 1.0];
        m
    }

    fn rotation_x(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::identity();
        m.0[1] = [0.0, c, s, 0.0];
        m.0[2] = [0.0, -s, c, 0.0];
        m
    }

    fn rotation_y(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::identity();
        m.0[0] = [c, 0.0, -s, 0.0];
        m.0[2] = [s, 0.0, c, 0.0];
        m
    }

    fn rotation_z(angle: f32) -> Self {
        let (s, c) = angle.sin_cos();
        let mut m = Self::identity();
        m.0[0] = [c, s, 0.0, 0.0];
        m.0[1] = [-s, c, 0.0, 0.0];
        m
    }

    /// Matrix product `self · other`, i.e. apply `other` first then `self`
    /// when transforming a column vector.
    pub(crate) fn mul(&self, other: &Self) -> Self {
        let mut out = [[0.0_f32; 4]; 4];
        // The element-by-element layout (`out[col][row]`) is the algorithm,
        // so iter_mut+enumerate doesn't read any clearer than indexed loops.
        #[allow(clippy::needless_range_loop)]
        for col in 0..4 {
            for row in 0..4 {
                let mut s = 0.0;
                for k in 0..4 {
                    s += self.0[k][row] * other.0[col][k];
                }
                out[col][row] = s;
            }
        }
        Self(out)
    }

    /// Transform a 3D point `(x, y, z)` through this matrix, with implicit
    /// `w = 1`. The v0 subset is affine (translation / rotation), so `w'`
    /// is always `1` and the homogeneous divide is skipped.
    pub(crate) fn transform_point(&self, p: [f32; 3]) -> [f32; 3] {
        let [x, y, z] = p;
        let m = &self.0;
        [
            m[0][0] * x + m[1][0] * y + m[2][0] * z + m[3][0],
            m[0][1] * x + m[1][1] * y + m[2][1] * z + m[3][1],
            m[0][2] * x + m[1][2] * y + m[2][2] * z + m[3][2],
        ]
    }
}

/// Parse a `transform` attribute value into a composed [`Mat4`].
///
/// Returns `Some(identity)` for an empty / whitespace-only value, and
/// `None` if any function is unrecognised or malformed — matching SPEC
/// §2.4 ("a `transform` value containing an unrecognised function is in
/// error; the affected attribute is ignored").
pub(crate) fn parse_transform(value: &str) -> Option<Mat4> {
    let mut c = Cursor::new(value);
    let mut acc = Mat4::identity();
    c.skip_ws();
    while !c.at_end() {
        let name = c.read_ident()?;
        c.skip_ws();
        c.expect(b'(')?;
        c.skip_ws();
        let m = match name {
            "translate" => {
                let tx = c.read_number()?;
                c.skip_ws();
                let ty = if c.peek() == Some(b')') {
                    0.0
                } else {
                    let v = c.read_number()?;
                    c.skip_ws();
                    v
                };
                Mat4::translation(tx, ty, 0.0)
            }
            "translate3d" => {
                let tx = c.read_number()?;
                c.skip_ws();
                let ty = c.read_number()?;
                c.skip_ws();
                let tz = c.read_number()?;
                c.skip_ws();
                Mat4::translation(tx, ty, tz)
            }
            "translateZ" => {
                let tz = c.read_number()?;
                c.skip_ws();
                Mat4::translation(0.0, 0.0, tz)
            }
            "rotate" => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::rotation_z(a)
            }
            "rotateX" => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::rotation_x(a)
            }
            "rotateY" => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::rotation_y(a)
            }
            "rotateZ" => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::rotation_z(a)
            }
            _ => return None,
        };
        c.expect(b')')?;
        c.skip_ws();
        acc = acc.mul(&m);
    }
    Some(acc)
}

struct Cursor<'a> {
    src: &'a [u8],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn new(src: &'a str) -> Self {
        Self {
            src: src.as_bytes(),
            pos: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.src.get(self.pos).copied()
    }

    fn at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn expect(&mut self, c: u8) -> Option<()> {
        if self.peek()? == c {
            self.pos += 1;
            Some(())
        } else {
            None
        }
    }

    fn skip_ws(&mut self) {
        while let Some(b) = self.peek() {
            if b.is_ascii_whitespace() || b == b',' {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn read_ident(&mut self) -> Option<&'a str> {
        let start = self.pos;
        // First char must be a letter; following chars may be alphanumeric
        // so identifiers like `translate3d` parse intact.
        if !matches!(self.peek(), Some(b) if b.is_ascii_alphabetic()) {
            return None;
        }
        self.pos += 1;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphanumeric() {
                self.pos += 1;
            } else {
                break;
            }
        }
        std::str::from_utf8(&self.src[start..self.pos]).ok()
    }

    fn read_number(&mut self) -> Option<f32> {
        let start = self.pos;
        if matches!(self.peek(), Some(b'+' | b'-')) {
            self.pos += 1;
        }
        let mut saw_digit = false;
        while let Some(b) = self.peek() {
            if b.is_ascii_digit() {
                self.pos += 1;
                saw_digit = true;
            } else {
                break;
            }
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    self.pos += 1;
                    saw_digit = true;
                } else {
                    break;
                }
            }
        }
        if !saw_digit {
            return None;
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            let exp_start = self.pos;
            while let Some(b) = self.peek() {
                if b.is_ascii_digit() {
                    self.pos += 1;
                } else {
                    break;
                }
            }
            if self.pos == exp_start {
                return None;
            }
        }
        let s = std::str::from_utf8(&self.src[start..self.pos]).ok()?;
        s.parse::<f32>().ok().filter(|v| v.is_finite())
    }

    /// Read a number followed by an optional angle unit (`deg` default,
    /// also `rad`, `turn`, `grad` per CSS-VALUES-4). Returns the angle in
    /// radians.
    fn read_angle(&mut self) -> Option<f32> {
        let n = self.read_number()?;
        let unit_start = self.pos;
        while let Some(b) = self.peek() {
            if b.is_ascii_alphabetic() {
                self.pos += 1;
            } else {
                break;
            }
        }
        let unit = std::str::from_utf8(&self.src[unit_start..self.pos]).ok()?;
        let radians = match unit {
            "" | "deg" => n.to_radians(),
            "rad" => n,
            "turn" => n * 2.0 * PI,
            "grad" => n * PI / 200.0,
            _ => return None,
        };
        Some(radians)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: [f32; 3], b: [f32; 3]) -> bool {
        let eps = 1e-4;
        (a[0] - b[0]).abs() < eps && (a[1] - b[1]).abs() < eps && (a[2] - b[2]).abs() < eps
    }

    #[test]
    fn empty_value_parses_to_identity() {
        let m = parse_transform("").unwrap();
        assert_eq!(m, Mat4::identity());
        let m = parse_transform("   ").unwrap();
        assert_eq!(m, Mat4::identity());
    }

    #[test]
    fn translate3d_offsets_point() {
        let m = parse_transform("translate3d(10, 20, 30)").unwrap();
        let p = m.transform_point([1.0, 2.0, 3.0]);
        assert!(approx(p, [11.0, 22.0, 33.0]));
    }

    #[test]
    fn translate_z_offsets_z_only() {
        let m = parse_transform("translateZ(5)").unwrap();
        let p = m.transform_point([1.0, 2.0, 0.0]);
        assert!(approx(p, [1.0, 2.0, 5.0]));
    }

    #[test]
    fn translate_2d_short_form() {
        // SVG 1.1 translate(tx) — `ty` defaults to 0.
        let m = parse_transform("translate(7)").unwrap();
        let p = m.transform_point([1.0, 2.0, 3.0]);
        assert!(approx(p, [8.0, 2.0, 3.0]));
        // translate(tx, ty)
        let m = parse_transform("translate(7, 4)").unwrap();
        let p = m.transform_point([1.0, 2.0, 3.0]);
        assert!(approx(p, [8.0, 6.0, 3.0]));
    }

    #[test]
    fn rotate_z_default_deg() {
        let m = parse_transform("rotate(90)").unwrap();
        let p = m.transform_point([1.0, 0.0, 0.0]);
        assert!(approx(p, [0.0, 1.0, 0.0]));
    }

    #[test]
    fn rotate_x_90_deg() {
        let m = parse_transform("rotateX(90deg)").unwrap();
        let p = m.transform_point([0.0, 1.0, 0.0]);
        // Per SPEC §4.2.5 the rotation matrix sends (0,1,0) to (0,0,1).
        assert!(approx(p, [0.0, 0.0, 1.0]));
    }

    #[test]
    fn rotate_y_90_deg_sends_x_to_negative_z() {
        let m = parse_transform("rotateY(90deg)").unwrap();
        let p = m.transform_point([1.0, 0.0, 0.0]);
        assert!(approx(p, [0.0, 0.0, -1.0]));
    }

    #[test]
    fn rotate_radians_unit() {
        let m = parse_transform("rotateZ(1.5707963rad)").unwrap();
        let p = m.transform_point([1.0, 0.0, 0.0]);
        assert!(approx(p, [0.0, 1.0, 0.0]));
    }

    #[test]
    fn translate3d_then_rotate_y_left_to_right() {
        // SPEC §4.3: transform="A B" → M = A · B, and (A · B) · p applies B first.
        let m = parse_transform("translate3d(0,0,10) rotateY(90deg)").unwrap();
        let p = m.transform_point([1.0, 0.0, 0.0]);
        // rotateY(90°) sends (1,0,0) to (0,0,-1); translate3d(0,0,10) sends
        // that to (0,0,9).
        assert!(approx(p, [0.0, 0.0, 9.0]));
    }

    #[test]
    fn unrecognised_function_rejects_whole_attribute() {
        assert!(parse_transform("matrix3d(1,0,0,0,0,1,0,0,0,0,1,0,0,0,0,1)").is_none());
        assert!(parse_transform("scale(2)").is_none());
        assert!(parse_transform("translateZ(5) skewX(10)").is_none());
    }

    #[test]
    fn malformed_input_rejects() {
        assert!(parse_transform("translate3d(1,2)").is_none()); // missing arg
        assert!(parse_transform("translateZ(").is_none()); // missing closing paren
        assert!(parse_transform("rotateX(90xx)").is_none()); // bad angle unit
    }
}
