# svg3
## A 3D Extension to Scalable Vector Graphics (SVG) 1.1

**Editor's draft.** This document defines `svg3`, an extension to
[Scalable Vector Graphics (SVG) 1.1](https://www.w3.org/TR/SVG11/)
that adds three-dimensional graphics elements, additional transform
functions, and the rendering model for them. It is not a W3C
publication. Issues are tracked at <https://github.com/PupilTong/svg3>.

## Table of contents

1. [Introduction](#1-introduction)
2. [Conformance](#2-conformance)
3. [Coordinate system and units](#3-coordinate-system-and-units)
4. [Transforms](#4-transforms)
5. [Three-dimensional graphics elements](#5-three-dimensional-graphics-elements)
6. [Painting and visibility](#6-painting-and-visibility)
7. [Rendering model](#7-rendering-model)
8. [References](#8-references)

---

## 1. Introduction

### 1.1 Overview

`svg3` extends SVG 1.1 with the ability to describe three-dimensional
graphics. An `svg3` document is a well-formed XML document with an
`'svg'` root element ([SVG11], §5.1), that may use the additional
elements and attribute values defined in this specification.
Specifically, `svg3` adds:

- A set of *three-dimensional graphics elements* (§5), namely
  `'cube'` and `'ellipsoid'`, which describe geometry that extends
  along all three coordinate axes.
- A set of *additional transform functions* (§4) — `translate3d`,
  `translateZ`, `scale3d`, `scaleZ`, `rotateX`, `rotateY`,
  `rotateZ`, `rotate3d`, and `matrix3d` — that may appear in the
  `'transform'` attribute alongside the SVG 1.1 transform functions.
- An *extension of the user coordinate system* (§3) with a Z axis,
  such that SVG 1.1's two-dimensional content lies in the plane
  `z = 0`.
- A *rendering model* (§7) for three-dimensional content, including
  the role of the viewing transformation, which is supplied by the
  user agent and is not addressable from the document.

The following document fragment is a minimal example:

```xml
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 200 200">
  <cube cx="100" cy="100" cz="0" size="80"
        transform="rotateY(30deg) rotateX(20deg)" fill="orange"/>
</svg>
```

### 1.2 Relationship to SVG 1.1

`svg3` is an extension of SVG 1.1. Every conforming `svg3` document
is also a well-formed SVG 1.1 document fragment. The elements and
transform functions introduced here are foreign to SVG 1.1; in a
user agent that implements only SVG 1.1, they are treated as
unsupported elements or unrecognised transform functions and produce
no visible rendering — see §2.4 ("Compatibility with SVG 1.1
implementations").

This specification does **not** redefine, replace, or restrict any
feature of SVG 1.1. All elements, attributes, properties, and
processing rules of SVG 1.1 apply unchanged to an `svg3` document
unless this specification explicitly states otherwise.

### 1.3 Document conventions

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHALL NOT",
"SHOULD", "SHOULD NOT", "RECOMMENDED", "MAY", and "OPTIONAL" in this
document are to be interpreted as described in [RFC 2119] and
[RFC 8174] when, and only when, they appear in all capitals, as
shown here.

References of the form [SVG11], [CSS-TRANSFORMS-2], etc. resolve to
the entries in §8.

Element names appear in single quotes when referred to in prose
(e.g., the `'cube'` element). Attribute and property names appear in
single quotes similarly.

---

## 2. Conformance

### 2.1 Conformance criteria

This specification defines two classes of products:

- a **conforming `svg3` document**, and
- a **conforming `svg3` user agent** (also called an *implementation*).

### 2.2 Document conformance

A document is a *conforming `svg3` document* if it satisfies all of
the following:

1. It is a conforming SVG 1.1 document fragment ([SVG11], §2.3).
2. Every element from the set defined in §5 that appears in the
   document complies with the syntax and attribute requirements of
   the corresponding subsection.
3. Every occurrence of the additional transform functions defined in
   §4 within a `'transform'` attribute value complies with the
   grammar in §4.1 and the function definitions in §4.2.

### 2.3 Implementation conformance

A user agent is a *conforming `svg3` user agent* if it satisfies all
of the following:

1. It is a conforming SVG 1.1 *dynamic interactive* or *static* user
   agent ([SVG11], §2.4) — that is, it implements SVG 1.1 in full
   for at least one of those profiles.
2. It correctly processes every element defined in §5 according to
   the rules in §§5–7.
3. It correctly processes every additional transform function defined
   in §4 according to §§4.2 and 4.3.
4. It implements the rendering model in §7.

The viewing transformation (camera) used to project three-dimensional
content is implementation-defined (§7.1). A conforming user agent
MUST provide some viewing transformation; it MUST NOT take that
transformation from the document.

### 2.4 Compatibility with SVG 1.1 implementations

A user agent that implements SVG 1.1 but not `svg3` is expected to
process an `svg3` document as follows, by virtue of SVG 1.1's own
processing rules:

- Each three-dimensional graphics element (§5) is an unrecognised
  element. Per [SVG11], §5.1, unrecognised elements produce no
  rendering and their content is not processed for layout.
- Each additional transform function (§4) is an unrecognised function
  within a `'transform'` value. Per [SVG11], §7.6, a `'transform'`
  value containing an unrecognised function is in error; the affected
  attribute is ignored.

This degradation is not normative for SVG 1.1 implementations (which
are out of scope of this specification); it describes the expected
behaviour given SVG 1.1's existing rules.

---

## 3. Coordinate system and units

### 3.1 The user coordinate system

`svg3` extends the *initial user coordinate system* defined in
[SVG11], §7.2, with a third axis. In an `svg3` document:

- The X axis points to the right.
- The Y axis points down.
- The Z axis points toward the viewer (out of the screen).

This coordinate system is *left-handed*, consistent with
[CSS-TRANSFORMS-2], §3 ("3D Transform Rendering Model").

SVG 1.1 two-dimensional content lies in the plane `z = 0`. The
position, scaling, and rotation of two-dimensional elements
(including all SVG 1.1 graphics elements, the `'g'` element, and the
`'svg'` element itself) is unaffected by this extension.

The Z axis is referenced by:

- the `'cz'` and `'rz'`/`'depth'` attributes on three-dimensional
  graphics elements (§5);
- the transform functions defined in §4.

### 3.2 Units

Lengths along all three axes use the same units, defined by SVG 1.1
([SVG11], §7.10). One *user unit* is the same length regardless of
axis. The mapping from user units to physical pixels is determined
by the SVG 1.1 outermost `'svg'` element's `'viewBox'`, `'width'`,
`'height'`, and `'preserveAspectRatio'` attributes (for the X and Y
axes) and by the viewing transformation (for the Z axis; §7).

The length-with-unit grammar of SVG 1.1 ([SVG11], §4.2) applies
unchanged to the new attributes defined in §5 (`'cz'`, `'depth'`,
`'rz'`, etc.).

---

## 4. Transforms

### 4.1 Extended transform-list grammar

`svg3` extends the grammar of the `'transform'` attribute defined in
[SVG11], §7.6, by adding the function names listed in §4.2 to the
set of permitted *transform commands*. The remainder of the grammar
— whitespace, separators, the comma policy, and the left-to-right
composition rule — is unchanged.

Informally:

```
transform-list ::= transform ( comma-wsp* transform )*
transform      ::= matrix  | translate  | scale  | rotate
                 | skewX   | skewY                          // SVG 1.1
                 | matrix3d | translate3d | translateZ
                 | scale3d  | scaleZ
                 | rotateX  | rotateY     | rotateZ
                 | rotate3d                                 // svg3
```

A `'transform'` attribute value containing both SVG 1.1 functions and
the functions defined here is well-formed. Functions of either kind
MAY appear in any order, and MAY be mixed in a single attribute.

### 4.2 Transform functions

Each transform function defines a 4×4 transformation matrix that
operates on column vectors `(x, y, z, 1)ᵀ` in homogeneous
coordinates. Angles accept the suffixes defined in [CSS-VALUES-4],
§7 (`deg`, `rad`, `turn`, `grad`); the default suffix is `deg`.

#### 4.2.1 `translate3d(tx, ty, tz)`

Translates by the vector `(tx, ty, tz)`. The corresponding matrix is:

```
| 1  0  0  tx |
| 0  1  0  ty |
| 0  0  1  tz |
| 0  0  0  1  |
```

#### 4.2.2 `translateZ(tz)`

Equivalent to `translate3d(0, 0, tz)`.

#### 4.2.3 `scale3d(sx, sy, sz)`

Scales by `(sx, sy, sz)`. The corresponding matrix is:

```
| sx 0  0  0 |
| 0  sy 0  0 |
| 0  0  sz 0 |
| 0  0  0  1 |
```

#### 4.2.4 `scaleZ(sz)`

Equivalent to `scale3d(1, 1, sz)`.

#### 4.2.5 `rotateX(angle)`

Rotates by `angle` about the X axis. With `c = cos(angle)` and
`s = sin(angle)`, the corresponding matrix is:

```
| 1  0  0  0 |
| 0  c  -s 0 |
| 0  s  c  0 |
| 0  0  0  1 |
```

#### 4.2.6 `rotateY(angle)`

Rotates by `angle` about the Y axis. With `c = cos(angle)` and
`s = sin(angle)`, the corresponding matrix is:

```
|  c  0  s  0 |
|  0  1  0  0 |
| -s  0  c  0 |
|  0  0  0  1 |
```

#### 4.2.7 `rotateZ(angle)`

Rotates by `angle` about the Z axis. With `c = cos(angle)` and
`s = sin(angle)`, the corresponding matrix is:

```
| c  -s 0  0 |
| s  c  0  0 |
| 0  0  1  0 |
| 0  0  0  1 |
```

`rotateZ(angle)` is equivalent to the SVG 1.1 function
`rotate(angle)` (interpreted with the origin of rotation at
`(0, 0)` and z-coordinate unchanged).

#### 4.2.8 `rotate3d(x, y, z, angle)`

Rotates by `angle` about the axis `(x, y, z)`. The axis is
normalised; if `(x, y, z)` is the zero vector, the function is in
error and the entire `'transform'` value is ignored. With
`c = cos(angle)`, `s = sin(angle)`, and `t = 1 - c`, and with
`(x, y, z)` having been normalised to unit length, the corresponding
matrix is:

```
| txx + c    txy - sz   txz + sy  0 |
| txy + sz   tyy + c    tyz - sx  0 |
| txz - sy   tyz + sx   tzz + c   0 |
| 0          0          0         1 |
```

(where `txy` denotes `t·x·y`, etc.).

#### 4.2.9 `matrix3d(m11, m12, m13, m14, m21, …, m44)`

Applies the 4×4 matrix whose elements are taken in **column-major**
order:

```
| m11 m21 m31 m41 |
| m12 m22 m32 m42 |
| m13 m23 m33 m43 |
| m14 m24 m34 m44 |
```

This is the same convention used by `matrix3d()` in
[CSS-TRANSFORMS-2], §11.

### 4.3 Composition

Transforms compose by matrix multiplication, applied left to right.
Given `transform="A B C"`, the resulting transformation matrix is
`A · B · C`, and a point `p` in the local coordinate system is
mapped to the surrounding coordinate system by `(A · B · C) · p`.
This matches [SVG11], §7.6.

Where a `'transform'` attribute is set on a descendant element, the
descendant's transformation is composed with its ancestors' by
matrix multiplication, applied from the root toward the leaf. This
is the same rule SVG 1.1 uses for two-dimensional transforms,
unchanged.

---

## 5. Three-dimensional graphics elements

The elements defined in this section are *graphics elements* in the
sense of [SVG11], §1.3. They may appear wherever an SVG 1.1
graphics element may appear.

### 5.1 Common attributes

Each three-dimensional graphics element accepts the following
attribute groups, defined by SVG 1.1:

- *Core attributes* ([SVG11], §5.2.1): `'id'`, `'xml:base'`,
  `'xml:lang'`, `'xml:space'`.
- *Conditional processing attributes* ([SVG11], §5.8.2):
  `'requiredFeatures'`, `'requiredExtensions'`, `'systemLanguage'`.
- *Style attributes*: `'class'`, `'style'` ([SVG11], §6.13).
- The `'transform'` attribute ([SVG11], §7.6), extended per §4.
- The presentation attributes corresponding to the properties
  defined in §6.

### 5.2 The `'cube'` element

The `'cube'` element defines an axis-aligned rectangular cuboid in
the user coordinate system.

**Attributes:**

| Attribute | Type | Default | Description |
|---|---|---|---|
| `cx` | `<coordinate>` | `0` | X coordinate of the centre. |
| `cy` | `<coordinate>` | `0` | Y coordinate of the centre. |
| `cz` | `<coordinate>` | `0` | Z coordinate of the centre. |
| `size` | `<length>` | (none) | Edge length along all three axes. |
| `width` | `<length>` | `size` if present, otherwise `0` | Edge length along X. |
| `height` | `<length>` | `size` if present, otherwise `0` | Edge length along Y. |
| `depth` | `<length>` | `size` if present, otherwise `0` | Edge length along Z. |

A negative value for `'size'`, `'width'`, `'height'`, or `'depth'`
is an error. The element MUST be rendered as if it had not been
specified.

A value of zero for `'width'`, `'height'`, or `'depth'` disables
rendering of the element, but does not disable rendering of its
descendants.

If `'width'`, `'height'`, or `'depth'` is not specified and `'size'`
is, the unspecified attribute defaults to the value of `'size'`. If
neither is specified, the per-axis attribute defaults to zero.

The element describes the axis-aligned cuboid whose centre, in local
coordinates, is `(cx, cy, cz)` and whose extent along each axis is
the corresponding attribute. The element's `'transform'` is then
applied, mapping the cuboid into the surrounding coordinate system
per §4.3.

### 5.3 The `'ellipsoid'` element

The `'ellipsoid'` element defines an ellipsoid in the user
coordinate system.

**Attributes:**

| Attribute | Type | Default | Description |
|---|---|---|---|
| `cx` | `<coordinate>` | `0` | X coordinate of the centre. |
| `cy` | `<coordinate>` | `0` | Y coordinate of the centre. |
| `cz` | `<coordinate>` | `0` | Z coordinate of the centre. |
| `r` | `<length>` | (none) | Radius along all three axes. |
| `rx` | `<length>` | `r` if present, otherwise `0` | Radius along X. |
| `ry` | `<length>` | `r` if present, otherwise `0` | Radius along Y. |
| `rz` | `<length>` | `r` if present, otherwise `0` | Radius along Z. |

A negative value for `'r'`, `'rx'`, `'ry'`, or `'rz'` is an error.
The element MUST be rendered as if it had not been specified.

A value of zero for `'rx'`, `'ry'`, or `'rz'` disables rendering of
the element.

If `'rx'`, `'ry'`, or `'rz'` is not specified and `'r'` is, the
unspecified attribute defaults to the value of `'r'`. If neither is
specified, the per-axis attribute defaults to zero.

The element describes the set of points `(x, y, z)` satisfying

```
((x − cx) / rx)² + ((y − cy) / ry)² + ((z − cz) / rz)² ≤ 1
```

(with all three radii strictly positive — see the zero-disables rule
above) in local coordinates. The element's `'transform'` is then
applied per §4.3.

The `'ellipsoid'` element does not replace SVG 1.1's `'ellipse'`
element ([SVG11], §9.4); two-dimensional ellipses in the plane
`z = 0` are authored with the SVG 1.1 `'ellipse'` element, and the
two are processed independently.

---

## 6. Painting and visibility

Three-dimensional graphics elements (§5) are painted using a subset
of the SVG 1.1 paint properties. This specification does not define
new paint properties.

### 6.1 The `'fill'` property

The SVG 1.1 `'fill'` property ([SVG11], §11.3) applies to
three-dimensional graphics elements. It specifies the paint used to
render the visible surface of the element.

When the computed value of `'fill'` is a `<paint>` value that
references a paint server (e.g., a gradient or pattern via
`url(#…)`), the rendering of the surface using that paint server is
implementation-defined. Conforming implementations SHOULD apply the
paint server's two-dimensional paint to the projected surface in a
manner consistent with that paint server's intent on a comparable
2D shape; the spec does not further constrain this mapping.

### 6.2 The `'opacity'` property

The CSS `'opacity'` property ([CSS-COLOR-4], §1.1, by reference from
SVG 1.1) applies to three-dimensional graphics elements with its
usual semantics.

### 6.3 Properties not defined by this specification

This specification does not define behaviour for the following SVG
1.1 paint or rendering properties when set on a three-dimensional
graphics element:

- `'stroke'` and the stroke-related properties (`'stroke-width'`,
  `'stroke-linecap'`, etc.).
- `'fill-rule'`.
- `'marker'` and related properties.
- `'clip-path'`, `'mask'`, `'filter'`.

Implementations MAY ignore these properties on three-dimensional
graphics elements, or MAY assign them an implementation-defined
meaning. Authors SHOULD NOT rely on any particular interpretation.

---

## 7. Rendering model

### 7.1 The viewing transformation

A conforming implementation projects three-dimensional content to
the two-dimensional viewport using an *implementation-defined viewing
transformation*. The viewing transformation comprises a position, an
orientation, and a projection (typically a perspective or
orthographic projection).

The viewing transformation is **not** addressable from the document.
This specification defines no element, attribute, or property by
which an author can set the viewing transformation. Implementations
MUST NOT consult the document for this information.

The viewing transformation MAY depend on user interaction (e.g.,
orbiting controls), on the embedding context (e.g., an AR/VR
session), or on implementation-defined heuristics (e.g., framing the
axis-aligned bounding box of the document's three-dimensional
content). The choice of default is implementation-defined.

### 7.2 Projection

Each three-dimensional graphics element is converted, after the
composition of its `'transform'` attribute with its ancestors'
transforms (§4.3), into a set of surface points in user-space
coordinates. The viewing transformation (§7.1) projects these points
to the two-dimensional viewport, producing pixel positions.

The surface is then filled with the computed value of the `'fill'`
property (§6.1), modulated by `'opacity'`.

### 7.3 Composition with SVG 1.1 content

Two-dimensional SVG 1.1 content lies in the plane `z = 0` (§3.1).
Three-dimensional graphics elements with all surface points
satisfying `z = 0` after composition of transforms produce
two-dimensional output and composite with SVG 1.1 content per
SVG 1.1's painter's algorithm ([SVG11], §3).

For three-dimensional graphics elements with surface points at
`z ≠ 0`, the compositing of those elements with one another and with
two-dimensional SVG 1.1 content is implementation-defined.
Implementations SHOULD use a depth-aware rendering technique
(e.g., a depth buffer, depth sorting, order-independent
transparency) such that elements with greater Z (closer to the
viewer) occlude elements with lesser Z.

---

## 8. References

### 8.1 Normative references

- **[SVG11]** Erik Dahlström et al., *Scalable Vector Graphics (SVG)
  1.1 (Second Edition)*. W3C Recommendation, 16 August 2011.
  <https://www.w3.org/TR/SVG11/>
- **[CSS-TRANSFORMS-2]** Tab Atkins Jr. et al., *CSS Transforms
  Module Level 2*. W3C Working Draft.
  <https://www.w3.org/TR/css-transforms-2/>
- **[CSS-VALUES-4]** Tab Atkins Jr. et al., *CSS Values and Units
  Module Level 4*. W3C Working Draft.
  <https://www.w3.org/TR/css-values-4/>
- **[CSS-COLOR-4]** Tab Atkins Jr. et al., *CSS Color Module Level
  4*. W3C Candidate Recommendation.
  <https://www.w3.org/TR/css-color-4/>
- **[RFC 2119]** S. Bradner, *Key words for use in RFCs to Indicate
  Requirement Levels*. IETF BCP 14, March 1997.
  <https://www.rfc-editor.org/rfc/rfc2119>
- **[RFC 8174]** B. Leiba, *Ambiguity of Uppercase vs Lowercase in
  RFC 2119 Key Words*. IETF BCP 14, May 2017.
  <https://www.rfc-editor.org/rfc/rfc8174>
