//! [SPEC.md](../../SPEC.md) §4 transform-attribute parser (v0 subset).
//!
//! Shared by `<surface>`'s child `<path>` handling and the scene walker's
//! global transform pipeline, which composes `transform` attributes onto
//! rendered elements per SPEC §4.3.
//!
//! Supported functions:
//!   * SVG 1.1 §7.6: `translate`, `scale`, `rotate(angle [, cx, cy])`,
//!     `skewX`, `skewY`, `matrix`.
//!   * SPEC §4.2: `translate3d`, `translateZ`, `scale3d`, `scaleZ`,
//!     `rotateX`, `rotateY`, `rotateZ`, `rotate3d`, `matrix3d`.
//!
//! Any other function name (or any malformed token) causes the parse
//! to fail; per SPEC §2.4 the whole attribute is then in error and
//! the caller MUST ignore it.

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

    fn scaling(sx: f32, sy: f32, sz: f32) -> Self {
        let mut m = [[0.0_f32; 4]; 4];
        m[0][0] = sx;
        m[1][1] = sy;
        m[2][2] = sz;
        m[3][3] = 1.0;
        Self(m)
    }

    /// SVG 1.1 §7.6 2D affine `matrix(a b c d e f)`. Sends
    /// `(x, y)` to `(a·x + c·y + e, b·x + d·y + f)`; `z` is preserved.
    fn matrix_2d(a: f32, b: f32, c: f32, d: f32, e: f32, f: f32) -> Self {
        Self([
            [a, b, 0.0, 0.0],
            [c, d, 0.0, 0.0],
            [0.0, 0.0, 1.0, 0.0],
            [e, f, 0.0, 1.0],
        ])
    }

    /// Build directly from 16 column-major entries, matching SPEC
    /// §4.2.9's `matrix3d(m11 m12 … m44)` convention.
    fn from_column_major(vals: &[f32; 16]) -> Self {
        Self([
            [vals[0], vals[1], vals[2], vals[3]],
            [vals[4], vals[5], vals[6], vals[7]],
            [vals[8], vals[9], vals[10], vals[11]],
            [vals[12], vals[13], vals[14], vals[15]],
        ])
    }

    /// `skewX(angle)` from SVG 1.1 §7.6 — shears `x` by `tan(angle)·y`.
    fn skew_x(angle: f32) -> Self {
        let mut m = Self::identity();
        m.0[1][0] = angle.tan();
        m
    }

    /// `skewY(angle)` from SVG 1.1 §7.6 — shears `y` by `tan(angle)·x`.
    fn skew_y(angle: f32) -> Self {
        let mut m = Self::identity();
        m.0[0][1] = angle.tan();
        m
    }

    /// SPEC §4.2.8 `rotate3d(x, y, z, angle)`. Returns `None` if the
    /// axis is the zero vector (SPEC §4.2.8: "the function is in
    /// error and the entire `'transform'` value is ignored").
    fn rotation_3d(x: f32, y: f32, z: f32, angle: f32) -> Option<Self> {
        let len = (x * x + y * y + z * z).sqrt();
        if !(len > 1e-10 && len.is_finite()) {
            return None;
        }
        let x = x / len;
        let y = y / len;
        let z = z / len;
        let (s, c) = angle.sin_cos();
        let t = 1.0 - c;
        Some(Self([
            // col 0 — coefficients applied to the input x.
            [t * x * x + c, t * x * y + s * z, t * x * z - s * y, 0.0],
            // col 1 — coefficients applied to the input y.
            [t * x * y - s * z, t * y * y + c, t * y * z + s * x, 0.0],
            // col 2 — coefficients applied to the input z.
            [t * x * z + s * y, t * y * z - s * x, t * z * z + c, 0.0],
            // col 3 — translation (none for a rotation).
            [0.0, 0.0, 0.0, 1.0],
        ]))
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

/// Parse an svg3 `transform` attribute value into a composed [`Mat4`].
///
/// Returns `Some(identity)` for an empty / whitespace-only value, and
/// `None` if any function is unrecognised or malformed — matching SPEC
/// §2.4 ("a `transform` value containing an unrecognised function is in
/// error; the affected attribute is ignored").
pub(crate) fn parse_transform(value: &str) -> Option<Mat4> {
    parse_transform_impl(value, true)
}

/// Parse a plain SVG 1.1 `transform` attribute.
///
/// This accepts only SVG 1.1 transform functions. svg3's 3D functions are
/// treated as unrecognised, so the whole attribute is ignored when they
/// appear in a document that has not opted into the extension.
pub(crate) fn parse_svg_transform(value: &str) -> Option<Mat4> {
    parse_transform_impl(value, false)
}

fn parse_transform_impl(value: &str, allow_3d: bool) -> Option<Mat4> {
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
            "translate3d" if allow_3d => {
                let tx = c.read_number()?;
                c.skip_ws();
                let ty = c.read_number()?;
                c.skip_ws();
                let tz = c.read_number()?;
                c.skip_ws();
                Mat4::translation(tx, ty, tz)
            }
            "translateZ" if allow_3d => {
                let tz = c.read_number()?;
                c.skip_ws();
                Mat4::translation(0.0, 0.0, tz)
            }
            "rotate" => {
                // SVG 1.1 §7.6: `rotate(angle [, cx, cy])`. With a
                // centre, equivalent to `translate(cx, cy) rotateZ(a)
                // translate(-cx, -cy)`.
                let a = c.read_angle()?;
                c.skip_ws();
                if c.peek() == Some(b')') {
                    Mat4::rotation_z(a)
                } else {
                    let cx = c.read_number()?;
                    c.skip_ws();
                    let cy = c.read_number()?;
                    c.skip_ws();
                    Mat4::translation(cx, cy, 0.0)
                        .mul(&Mat4::rotation_z(a))
                        .mul(&Mat4::translation(-cx, -cy, 0.0))
                }
            }
            "rotateX" if allow_3d => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::rotation_x(a)
            }
            "rotateY" if allow_3d => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::rotation_y(a)
            }
            "rotateZ" if allow_3d => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::rotation_z(a)
            }
            "rotate3d" if allow_3d => {
                let x = c.read_number()?;
                c.skip_ws();
                let y = c.read_number()?;
                c.skip_ws();
                let z = c.read_number()?;
                c.skip_ws();
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::rotation_3d(x, y, z, a)?
            }
            "scale" => {
                // SVG 1.1 §7.6: `scale(sx [, sy])`. With one arg,
                // `sy` defaults to `sx`.
                let sx = c.read_number()?;
                c.skip_ws();
                let sy = if c.peek() == Some(b')') {
                    sx
                } else {
                    let v = c.read_number()?;
                    c.skip_ws();
                    v
                };
                Mat4::scaling(sx, sy, 1.0)
            }
            "scale3d" if allow_3d => {
                let sx = c.read_number()?;
                c.skip_ws();
                let sy = c.read_number()?;
                c.skip_ws();
                let sz = c.read_number()?;
                c.skip_ws();
                Mat4::scaling(sx, sy, sz)
            }
            "scaleZ" if allow_3d => {
                let sz = c.read_number()?;
                c.skip_ws();
                Mat4::scaling(1.0, 1.0, sz)
            }
            "skewX" => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::skew_x(a)
            }
            "skewY" => {
                let a = c.read_angle()?;
                c.skip_ws();
                Mat4::skew_y(a)
            }
            "matrix" => {
                let a = c.read_number()?;
                c.skip_ws();
                let b = c.read_number()?;
                c.skip_ws();
                let cc = c.read_number()?;
                c.skip_ws();
                let d = c.read_number()?;
                c.skip_ws();
                let e = c.read_number()?;
                c.skip_ws();
                let f = c.read_number()?;
                c.skip_ws();
                Mat4::matrix_2d(a, b, cc, d, e, f)
            }
            "matrix3d" if allow_3d => {
                let mut vals = [0.0_f32; 16];
                for slot in &mut vals {
                    *slot = c.read_number()?;
                    c.skip_ws();
                }
                Mat4::from_column_major(&vals)
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
    fn plain_svg_transform_rejects_svg3_3d_functions() {
        assert!(parse_svg_transform("translate(7, 4) rotate(90)").is_some());
        assert!(parse_svg_transform("translate3d(1, 2, 3)").is_none());
        assert!(parse_svg_transform("rotateZ(90)").is_none());
        assert!(parse_svg_transform("translate(7, 4) translateZ(3)").is_none());
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
    fn scale_uniform_scales_xy_only() {
        let m = parse_transform("scale(2)").unwrap();
        let p = m.transform_point([1.0, 2.0, 3.0]);
        assert!(approx(p, [2.0, 4.0, 3.0]));
    }

    #[test]
    fn scale_xy_independent() {
        let m = parse_transform("scale(2, 3)").unwrap();
        let p = m.transform_point([1.0, 1.0, 1.0]);
        assert!(approx(p, [2.0, 3.0, 1.0]));
    }

    #[test]
    fn scale_3d_scales_z_too() {
        let m = parse_transform("scale3d(2, 3, 4)").unwrap();
        let p = m.transform_point([1.0, 1.0, 1.0]);
        assert!(approx(p, [2.0, 3.0, 4.0]));
    }

    #[test]
    fn scale_z_only_scales_z() {
        let m = parse_transform("scaleZ(5)").unwrap();
        let p = m.transform_point([2.0, 3.0, 4.0]);
        assert!(approx(p, [2.0, 3.0, 20.0]));
    }

    #[test]
    fn matrix_2d_translates_origin() {
        // matrix(1, 0, 0, 1, 10, 20) is a pure translation by (10, 20).
        let m = parse_transform("matrix(1, 0, 0, 1, 10, 20)").unwrap();
        let p = m.transform_point([0.0, 0.0, 0.0]);
        assert!(approx(p, [10.0, 20.0, 0.0]));
    }

    #[test]
    fn matrix3d_identity_is_noop() {
        let m = parse_transform("matrix3d(1,0,0,0, 0,1,0,0, 0,0,1,0, 0,0,0,1)").unwrap();
        let p = m.transform_point([7.0, -3.0, 11.0]);
        assert!(approx(p, [7.0, -3.0, 11.0]));
    }

    #[test]
    fn skew_x_shears_x_by_y() {
        // skewX(45°) sends (0, 1, 0) → (1, 1, 0).
        let m = parse_transform("skewX(45deg)").unwrap();
        let p = m.transform_point([0.0, 1.0, 0.0]);
        assert!(approx(p, [1.0, 1.0, 0.0]));
    }

    #[test]
    fn skew_y_shears_y_by_x() {
        let m = parse_transform("skewY(45deg)").unwrap();
        let p = m.transform_point([1.0, 0.0, 0.0]);
        assert!(approx(p, [1.0, 1.0, 0.0]));
    }

    #[test]
    fn rotate3d_about_y_axis_matches_rotate_y() {
        // `rotate3d(0, 1, 0, 90deg)` should match `rotateY(90deg)`.
        let r3d = parse_transform("rotate3d(0, 1, 0, 90deg)").unwrap();
        let ry = parse_transform("rotateY(90deg)").unwrap();
        for &p in &[[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]] {
            assert!(approx(r3d.transform_point(p), ry.transform_point(p)));
        }
    }

    #[test]
    fn rotate3d_zero_axis_rejects() {
        // SPEC §4.2.8: the zero vector is in error.
        assert!(parse_transform("rotate3d(0, 0, 0, 90deg)").is_none());
    }

    #[test]
    fn rotate_with_centre_pivots_about_point() {
        // rotate(180, 10, 10) about (10, 10) sends (0, 0) → (20, 20).
        let m = parse_transform("rotate(180, 10, 10)").unwrap();
        let p = m.transform_point([0.0, 0.0, 0.0]);
        assert!(approx(p, [20.0, 20.0, 0.0]));
    }

    #[test]
    fn mixed_svg11_and_svg3_transforms_compose() {
        // SPEC §4.1: SVG 1.1 and svg3 transforms may appear together.
        let m = parse_transform("scale(2) translateZ(5)").unwrap();
        // Left-to-right: scale applied last (to the result of translateZ).
        let p = m.transform_point([1.0, 1.0, 0.0]);
        assert!(approx(p, [2.0, 2.0, 5.0]));
    }

    #[test]
    fn unrecognised_function_rejects_whole_attribute() {
        // None of these names are in the supported set.
        assert!(parse_transform("perspective(500)").is_none());
        assert!(parse_transform("translateZ(5) bogus(10)").is_none());
    }

    #[test]
    fn malformed_input_rejects() {
        assert!(parse_transform("translate3d(1,2)").is_none()); // missing arg
        assert!(parse_transform("translateZ(").is_none()); // missing closing paren
        assert!(parse_transform("rotateX(90xx)").is_none()); // bad angle unit
        assert!(parse_transform("matrix(1,2,3)").is_none()); // matrix needs 6 args
    }
}
