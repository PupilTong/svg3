//! SVG fixtures for the 3D crab snapshot tests.
//!
//! The character is built purely from svg3's three SPEC §5 primitives
//! (`<ellipsoid>`, `<cube>`, `<surface>`). All "3D" shading is faked
//! with one shared SVG filter chain that turns the flat silhouette's
//! alpha into a height field, then runs `feDiffuseLighting` +
//! `feSpecularLighting` over it — no scene-lighting feature is
//! required, per the agreed approach.
//!
//! **Z-ordering note.** Filter compositing writes the `z = 0` plane's
//! NDC depth at every pixel outside the source silhouette (see
//! `filter.wgsl::fs_composite` and the `plane_ndc_depth` fallback).
//! Under default `LessEqual` depth-compare this means a filtered
//! shape's quad will overwrite (with transparent color + the plane's
//! depth) any 3D geometry whose actual depth is greater than the plane
//! at that pixel. Under the default head-on ortho view that's mostly
//! invisible — alpha blending preserves colors — but under a perspective
//! camera + orbit, the `z = 0` plane tilts in world space and starts
//! slicing through any primitive whose back face sits behind it.
//!
//! To stay orbit-safe, every primitive in the crab is laid out so its
//! **back face** (`cz - r`) sits comfortably in front of `z = 0`. The
//! whole character is shifted to `cz ≥ ~70`; the body's back at `z = 5`
//! is the closest the model ever gets to the `z = 0` plane.
//!
//! 2D `<path>`/`<rect>` `transform` is not yet composed by the scene
//! walker, so a 2D path's z is fixed at the painter-order forward bias
//! (~0). With everything else now at `cz ≥ 70` a 2D path would render
//! far behind the model; the lightning-bolt decal is therefore authored
//! as a small `<cube>` in front of the hammer instead.

#![allow(dead_code)]

/// Render size (square) for [`SHADE_PROBE_SVG`].
pub const SHADE_PROBE_SIZE: u32 = 200;

/// The minimal probe: one `<ellipsoid>` with the shade filter applied,
/// validating that the filter pipeline produces fake-3D shading on a
/// 3D primitive's silhouette. This is the "smallest unit of evidence"
/// for the approach.
///
/// `cz` is pushed forward of the `z = 0` plane so the probe also reads
/// correctly under a perspective/orbit camera (see module doc).
pub const SHADE_PROBE_SVG: &str = r##"<svg width="200" height="200" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <filter id="shade-3d">
      <feGaussianBlur in="SourceAlpha" stdDeviation="5" result="bump"/>
      <feDiffuseLighting in="bump" surfaceScale="14" diffuseConstant="1"
                         lighting-color="#fff1d6" result="diffuse">
        <feDistantLight azimuth="135" elevation="55"/>
      </feDiffuseLighting>
      <feComposite in="diffuse" in2="SourceGraphic" operator="arithmetic"
                   k1="1" k2="0" k3="0" k4="0" result="shaded"/>
      <feSpecularLighting in="bump" surfaceScale="14" specularConstant="0.9"
                          specularExponent="22" lighting-color="#ffffff"
                          result="specular">
        <feDistantLight azimuth="135" elevation="55"/>
      </feSpecularLighting>
      <feComposite in="specular" in2="SourceGraphic" operator="in"
                   result="spec-in"/>
      <feComposite in="spec-in" in2="shaded" operator="arithmetic"
                   k1="0" k2="1" k3="1" k4="0"/>
    </filter>
  </defs>
  <ellipsoid cx="100" cy="100" cz="75" rx="70" ry="70" rz="70"
             fill="#f5a87a" filter="url(#shade-3d)"/>
</svg>"##;

/// Render size (square) for [`CRAB_FACE_SVG`].
pub const CRAB_FACE_SIZE: u32 = 300;

/// An intermediate milestone: the face — body + two oval eyes (each
/// with a dominant iris and a bright catchlight) + a wide mouth with
/// a visible tooth. Validates that the shading filter composes across
/// several adjacent ellipsoids and that the spatial Z layering (face
/// features at cz > body) reads correctly.
pub const CRAB_FACE_SVG: &str = r##"<svg width="300" height="300" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <filter id="shade-3d">
      <feGaussianBlur in="SourceAlpha" stdDeviation="5" result="bump"/>
      <feDiffuseLighting in="bump" surfaceScale="14" diffuseConstant="1"
                         lighting-color="#fff1d6" result="diffuse">
        <feDistantLight azimuth="135" elevation="55"/>
      </feDiffuseLighting>
      <feComposite in="diffuse" in2="SourceGraphic" operator="arithmetic"
                   k1="1" k2="0" k3="0" k4="0" result="shaded"/>
      <feSpecularLighting in="bump" surfaceScale="14" specularConstant="0.9"
                          specularExponent="22" lighting-color="#ffffff"
                          result="specular">
        <feDistantLight azimuth="135" elevation="55"/>
      </feSpecularLighting>
      <feComposite in="specular" in2="SourceGraphic" operator="in"
                   result="spec-in"/>
      <feComposite in="spec-in" in2="shaded" operator="arithmetic"
                   k1="0" k2="1" k3="1" k4="0"/>
    </filter>
  </defs>

  <!-- Body bulb: wide and squat. cz = 90, rz = 85 → back at z = 5. -->
  <ellipsoid cx="150" cy="170" cz="90" rx="115" ry="85" rz="85"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- Eyes: oval white sclera, dominant brown iris, bright catchlights -->
  <ellipsoid cx="118" cy="140" cz="150" rx="33" ry="38" rz="33"
             fill="#ffffff" filter="url(#shade-3d)"/>
  <ellipsoid cx="182" cy="140" cz="150" rx="33" ry="38" rz="33"
             fill="#ffffff" filter="url(#shade-3d)"/>

  <ellipsoid cx="112" cy="150" cz="195" rx="22" ry="26" rz="18"
             fill="#3a2418" filter="url(#shade-3d)"/>
  <ellipsoid cx="188" cy="150" cz="195" rx="22" ry="26" rz="18"
             fill="#3a2418" filter="url(#shade-3d)"/>

  <ellipsoid cx="106" cy="143" cz="220" rx="6" ry="7" rz="5"
             fill="#ffffff"/>
  <ellipsoid cx="182" cy="143" cz="220" rx="6" ry="7" rz="5"
             fill="#ffffff"/>
  <ellipsoid cx="118" cy="161" cz="220" rx="3" ry="3" rz="2"
             fill="#ffffff"/>
  <ellipsoid cx="194" cy="161" cz="220" rx="3" ry="3" rz="2"
             fill="#ffffff"/>

  <!-- Mouth: wide dark recess with a clearly visible tooth -->
  <ellipsoid cx="150" cy="200" cz="150" rx="32" ry="15" rz="6"
             fill="#1f1209" filter="url(#shade-3d)"/>
  <ellipsoid cx="135" cy="196" cz="158" rx="6" ry="8" rz="3"
             fill="#ffffff" filter="url(#shade-3d)"/>
</svg>"##;

/// Render size (square) for [`CRAB_FULL_SVG`].
pub const CRAB_FULL_SIZE: u32 = 400;

/// The full character. Composition reads roughly like the 2D reference:
/// cape flowing out on the lower-left, hammer floating above-left of the
/// head, body bulb filling the lower-middle, oval eyes with dominant
/// brown irises and bright catchlights, wide cartoon mouth with one
/// visible tooth, two pincer-claw arms (two ellipsoids each for the
/// split pincer), and a small white bauble cradled in the right claw.
///
/// All `cz` values are shifted so every primitive sits at `cz - r > 0`
/// (back face in front of the `z = 0` plane), so the model reads
/// correctly under both the default ortho view and a perspective +
/// orbit camera (see module doc).
pub const CRAB_FULL_SVG: &str = r##"<svg width="400" height="400" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <filter id="shade-3d">
      <feGaussianBlur in="SourceAlpha" stdDeviation="5" result="bump"/>
      <feDiffuseLighting in="bump" surfaceScale="14" diffuseConstant="1"
                         lighting-color="#fff1d6" result="diffuse">
        <feDistantLight azimuth="135" elevation="55"/>
      </feDiffuseLighting>
      <feComposite in="diffuse" in2="SourceGraphic" operator="arithmetic"
                   k1="1" k2="0" k3="0" k4="0" result="shaded"/>
      <feSpecularLighting in="bump" surfaceScale="14" specularConstant="0.9"
                          specularExponent="22" lighting-color="#ffffff"
                          result="specular">
        <feDistantLight azimuth="135" elevation="55"/>
      </feSpecularLighting>
      <feComposite in="specular" in2="SourceGraphic" operator="in"
                   result="spec-in"/>
      <feComposite in="spec-in" in2="shaded" operator="arithmetic"
                   k1="0" k2="1" k3="1" k4="0"/>
    </filter>
  </defs>

  <!-- Cape: lower-left, two stacked red blobs. cz < body cz so the
       cape sits behind the body in depth, but still in front of the
       z = 0 plane to stay orbit-safe. -->
  <ellipsoid cx="110" cy="305" cz="75" rx="48" ry="60" rz="13"
             fill="#c8242c" filter="url(#shade-3d)"/>
  <ellipsoid cx="95" cy="265" cz="75" rx="32" ry="40" rz="9"
             fill="#a01a22" filter="url(#shade-3d)"/>

  <!-- Hammer head: gray cube floating above-left of the body. -->
  <cube cx="180" cy="70" cz="80" size="70"
        fill="#d8d8d8" filter="url(#shade-3d)"/>

  <!-- Lightning-bolt decal: small yellow cube on the hammer's face.
       Hammer front at z = 80 + 35 = 115; bolt at cz = 130 size = 22
       → back z = 119, cleanly in front of the hammer. -->
  <cube cx="182" cy="68" cz="130" size="22"
        fill="#f5d33a" filter="url(#shade-3d)"/>

  <!-- Body bulb: wide squat peach. cz = 90, rz = 85 → back z = 5. -->
  <ellipsoid cx="210" cy="245" cz="90" rx="118" ry="88" rz="85"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- Bottom legs / pincer feet -->
  <ellipsoid cx="175" cy="325" cz="100" rx="11" ry="8" rz="8"
             fill="#c8242c" filter="url(#shade-3d)"/>
  <ellipsoid cx="245" cy="325" cz="100" rx="11" ry="8" rz="8"
             fill="#c8242c" filter="url(#shade-3d)"/>

  <!-- Left arm + pincer claw (upper + lower halves of the split pincer) -->
  <ellipsoid cx="108" cy="248" cz="105" rx="30" ry="24" rz="24"
             fill="#f5a87a" filter="url(#shade-3d)"/>
  <ellipsoid cx="72" cy="265" cz="115" rx="22" ry="13" rz="13"
             fill="#f5a87a" filter="url(#shade-3d)"/>
  <ellipsoid cx="68" cy="285" cz="115" rx="20" ry="13" rz="13"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- Right arm + pincer claw, mirrored -->
  <ellipsoid cx="320" cy="245" cz="105" rx="30" ry="24" rz="24"
             fill="#f5a87a" filter="url(#shade-3d)"/>
  <ellipsoid cx="360" cy="258" cz="115" rx="22" ry="13" rz="13"
             fill="#f5a87a" filter="url(#shade-3d)"/>
  <ellipsoid cx="362" cy="288" cz="115" rx="22" ry="13" rz="13"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- White bauble cradled in the right pincer, sitting in front -->
  <ellipsoid cx="375" cy="275" cz="135" rx="15" ry="15" rz="14"
             fill="#ffffff" filter="url(#shade-3d)"/>

  <!-- Eyes: oval sclera, dominant brown iris, primary + secondary
       catchlights. Pushed close enough that the two eyes touch in
       screen space — characteristic of the reference's "fused" look. -->
  <ellipsoid cx="172" cy="210" cz="150" rx="44" ry="50" rz="44"
             fill="#ffffff" filter="url(#shade-3d)"/>
  <ellipsoid cx="252" cy="210" cz="150" rx="44" ry="50" rz="44"
             fill="#ffffff" filter="url(#shade-3d)"/>

  <ellipsoid cx="163" cy="225" cz="195" rx="29" ry="34" rz="22"
             fill="#3a2418" filter="url(#shade-3d)"/>
  <ellipsoid cx="261" cy="225" cz="195" rx="29" ry="34" rz="22"
             fill="#3a2418" filter="url(#shade-3d)"/>

  <!-- Primary catchlights (upper-left of each iris, matches the
       filter's key-light direction) -->
  <ellipsoid cx="154" cy="213" cz="220" rx="8" ry="10" rz="6"
             fill="#ffffff"/>
  <ellipsoid cx="252" cy="213" cz="220" rx="8" ry="10" rz="6"
             fill="#ffffff"/>
  <!-- Secondary catchlights (smaller, lower-right of each iris) -->
  <ellipsoid cx="172" cy="242" cz="220" rx="3" ry="3" rz="2"
             fill="#ffffff"/>
  <ellipsoid cx="270" cy="242" cz="220" rx="3" ry="3" rz="2"
             fill="#ffffff"/>

  <!-- Mouth: wide dark recess with a clearly visible tooth on the left -->
  <ellipsoid cx="210" cy="280" cz="150" rx="38" ry="18" rz="7"
             fill="#1f1209" filter="url(#shade-3d)"/>
  <ellipsoid cx="190" cy="275" cz="158" rx="7" ry="9" rz="3"
             fill="#ffffff" filter="url(#shade-3d)"/>
</svg>"##;
