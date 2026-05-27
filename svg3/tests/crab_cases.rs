//! SVG fixtures for the 3D crab snapshot tests.
//!
//! The character is built purely from svg3's three SPEC §5 primitives
//! (`<ellipsoid>`, `<cube>`, `<surface>`) plus a small number of 2D
//! `<path>` decorations. All "3D" shading is faked with one shared SVG
//! filter chain that turns the flat silhouette's alpha into a height
//! field, then runs `feDiffuseLighting` + `feSpecularLighting` over it
//! — no scene-lighting feature is required, per the agreed approach.
//!
//! **Z-ordering note.** Filter compositing writes the `z = 0` plane's
//! NDC depth at every pixel outside the source silhouette (see
//! `filter.wgsl::fs_composite` and the `plane_ndc_depth` fallback).
//! Under default `LessEqual` depth-compare this means a filtered shape's
//! quad effectively occludes any geometry behind `z = 0`, so every 3D
//! primitive in the crab sits at `cz ≥ 0` and the cape — the only part
//! that needs to be "behind" the body — is laid out to extend outside
//! the body's screen-space silhouette rather than relying on a smaller
//! Z value. The lightning bolt sits in front of the hammer cube as a
//! small cube of its own (2D-path transforms are not yet composed by
//! the scene walker, so `translateZ` on a `<path>` is a no-op).

#![allow(dead_code)]

/// One reusable filter chain: blur SourceAlpha into a height field, run
/// a warm diffuse key + tight specular highlight, fold the diffuse into
/// the fill color, then add the specular on top. Same primitive list as
/// the original shade-on-sphere probe.
const SHADE_FILTER_DEFS: &str = r##"<defs>
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
  </defs>"##;

/// Render size (square) for [`SHADE_PROBE_SVG`].
pub const SHADE_PROBE_SIZE: u32 = 200;

/// The minimal probe: one `<ellipsoid>` with the shade filter applied,
/// validating that the filter pipeline produces fake-3D shading on a
/// 3D primitive's silhouette. This is the "smallest unit of evidence"
/// for the approach.
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
  <ellipsoid cx="100" cy="100" cz="0" rx="70" ry="70" rz="70"
             fill="#f5a87a" filter="url(#shade-3d)"/>
</svg>"##;

/// Render size (square) for [`CRAB_FACE_SVG`].
pub const CRAB_FACE_SIZE: u32 = 300;

/// An intermediate milestone: the head — body + two eyes (sclera, iris,
/// catchlight) + mouth (dark recess + tooth). Validates that the
/// shading filter composes across several adjacent ellipsoids and that
/// the spatial Z layering (face features at cz > body) reads correctly.
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

  <!-- Body bulb (peach) -->
  <ellipsoid cx="150" cy="160" cz="10" rx="100" ry="80" rz="80"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- Eyes: white sclera, dark iris, unfiltered bright catchlight -->
  <ellipsoid cx="115" cy="140" cz="60" rx="34" ry="36" rz="34"
             fill="#ffffff" filter="url(#shade-3d)"/>
  <ellipsoid cx="185" cy="140" cz="60" rx="34" ry="36" rz="34"
             fill="#ffffff" filter="url(#shade-3d)"/>
  <ellipsoid cx="110" cy="148" cz="90" rx="17" ry="17" rz="15"
             fill="#3a2418" filter="url(#shade-3d)"/>
  <ellipsoid cx="190" cy="148" cz="90" rx="17" ry="17" rz="15"
             fill="#3a2418" filter="url(#shade-3d)"/>
  <ellipsoid cx="104" cy="142" cz="108" rx="5" ry="5" rz="4"
             fill="#ffffff"/>
  <ellipsoid cx="184" cy="142" cz="108" rx="5" ry="5" rz="4"
             fill="#ffffff"/>

  <!-- Mouth: dark recess with one white tooth -->
  <ellipsoid cx="150" cy="195" cz="60" rx="22" ry="12" rz="5"
             fill="#1f1209" filter="url(#shade-3d)"/>
  <ellipsoid cx="142" cy="191" cz="66" rx="4" ry="5" rz="3"
             fill="#ffffff"/>
</svg>"##;

/// Render size (square) for [`CRAB_FULL_SVG`].
pub const CRAB_FULL_SIZE: u32 = 400;

/// The full character: cape, hammer (cube head + small yellow "bolt"
/// cube), body, arms, legs, face. Every 3D primitive uses the shared
/// `shade-3d` filter for fake-3D shading.
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

  <!-- Cape: positioned to extend OUTSIDE the body's screen silhouette
       (to the right and down), since shapes "fully behind" the body's
       filter region get color-preserved but depth-overwritten — fine
       for a static character but limits how much of the cape can hide
       behind the body. -->
  <ellipsoid cx="320" cy="290" cz="0" rx="55" ry="75" rz="14"
             fill="#c8242c" filter="url(#shade-3d)"/>

  <!-- Hammer head: gray cube floating above the body. -->
  <cube cx="200" cy="68" cz="0" size="64"
        fill="#cccccc" filter="url(#shade-3d)"/>
  <!-- "Lightning bolt" — a small yellow cube on the front face of the
       hammer head. Authored as a cube (3D) because 2D <path> transforms
       are not yet composed by the scene walker, so a 2D bolt at z = 0
       would be hidden behind the hammer cube's front face at z = +32. -->
  <cube cx="200" cy="65" cz="40" size="22"
        fill="#f5d33a" filter="url(#shade-3d)"/>

  <!-- Body bulb (peach) -->
  <ellipsoid cx="200" cy="230" cz="10" rx="105" ry="80" rz="80"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- Legs / pincer feet (red) -->
  <ellipsoid cx="160" cy="315" cz="20" rx="13" ry="9" rz="9"
             fill="#c8242c" filter="url(#shade-3d)"/>
  <ellipsoid cx="240" cy="315" cz="20" rx="13" ry="9" rz="9"
             fill="#c8242c" filter="url(#shade-3d)"/>

  <!-- Left arm + claw -->
  <ellipsoid cx="105" cy="240" cz="25" rx="28" ry="24" rz="24"
             fill="#f5a87a" filter="url(#shade-3d)"/>
  <ellipsoid cx="72" cy="270" cz="30" rx="24" ry="20" rz="20"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- Right arm + claw -->
  <ellipsoid cx="295" cy="240" cz="25" rx="28" ry="24" rz="24"
             fill="#f5a87a" filter="url(#shade-3d)"/>
  <ellipsoid cx="328" cy="270" cz="30" rx="24" ry="20" rz="20"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- Face: eyes (sclera + iris + catchlight) and mouth (recess + tooth).
       Face elements sit at higher cz than the body so their filter
       compositing depth wins LessEqual cleanly against the body. -->
  <ellipsoid cx="160" cy="205" cz="70" rx="40" ry="42" rz="40"
             fill="#ffffff" filter="url(#shade-3d)"/>
  <ellipsoid cx="240" cy="205" cz="70" rx="40" ry="42" rz="40"
             fill="#ffffff" filter="url(#shade-3d)"/>

  <ellipsoid cx="155" cy="215" cz="110" rx="20" ry="20" rz="18"
             fill="#3a2418" filter="url(#shade-3d)"/>
  <ellipsoid cx="245" cy="215" cz="110" rx="20" ry="20" rz="18"
             fill="#3a2418" filter="url(#shade-3d)"/>

  <ellipsoid cx="147" cy="207" cz="130" rx="6" ry="6" rz="5"
             fill="#ffffff"/>
  <ellipsoid cx="237" cy="207" cz="130" rx="6" ry="6" rz="5"
             fill="#ffffff"/>

  <ellipsoid cx="200" cy="265" cz="70" rx="24" ry="13" rz="6"
             fill="#1f1209" filter="url(#shade-3d)"/>
  <ellipsoid cx="190" cy="261" cz="76" rx="5" ry="6" rz="3"
             fill="#ffffff"/>
</svg>"##;
