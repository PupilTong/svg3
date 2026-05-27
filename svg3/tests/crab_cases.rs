//! SVG fixtures for the 3D crab snapshot tests.
//!
//! The character is built purely from svg3's three SPEC §5 primitives
//! (`<ellipsoid>`, `<cube>`, `<surface>`). All "3D" shading is faked
//! with one shared SVG filter chain that turns the flat silhouette's
//! alpha into a height field, then runs `feDiffuseLighting` +
//! `feSpecularLighting` over it — no scene-lighting feature is
//! required, per the agreed approach.

#![allow(dead_code)]

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

  <!-- Body bulb: wide and squat -->
  <ellipsoid cx="150" cy="170" cz="10" rx="115" ry="85" rz="85"
             fill="#f5a87a" filter="url(#shade-3d)"/>

  <!-- Eyes: oval white sclera, dominant brown iris, bright catchlights -->
  <ellipsoid cx="118" cy="140" cz="60" rx="33" ry="38" rz="33"
             fill="#ffffff" filter="url(#shade-3d)"/>
  <ellipsoid cx="182" cy="140" cz="60" rx="33" ry="38" rz="33"
             fill="#ffffff" filter="url(#shade-3d)"/>

  <ellipsoid cx="112" cy="150" cz="95" rx="22" ry="26" rz="18"
             fill="#3a2418" filter="url(#shade-3d)"/>
  <ellipsoid cx="188" cy="150" cz="95" rx="22" ry="26" rz="18"
             fill="#3a2418" filter="url(#shade-3d)"/>

  <ellipsoid cx="106" cy="143" cz="115" rx="6" ry="7" rz="5"
             fill="#ffffff"/>
  <ellipsoid cx="182" cy="143" cz="115" rx="6" ry="7" rz="5"
             fill="#ffffff"/>
  <ellipsoid cx="118" cy="161" cz="115" rx="3" ry="3" rz="2"
             fill="#ffffff"/>
  <ellipsoid cx="194" cy="161" cz="115" rx="3" ry="3" rz="2"
             fill="#ffffff"/>

  <!-- Mouth: wide dark recess with a clearly visible tooth -->
  <ellipsoid cx="150" cy="200" cz="60" rx="32" ry="15" rz="6"
             fill="#1f1209" filter="url(#shade-3d)"/>
  <ellipsoid cx="135" cy="196" cz="68" rx="6" ry="8" rz="3"
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
pub const CRAB_FULL_SVG: &str = r##"<svg width="400" height="400" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <!-- Soft / matte: diffuse only, no specular highlight. Used for
         the body, arms, legs, cape — the 2D reference has a soft peach
         gradient on the body, not a glossy ceramic highlight. -->
    <filter id="shade-soft">
      <feGaussianBlur in="SourceAlpha" stdDeviation="5" result="bump"/>
      <feDiffuseLighting in="bump" surfaceScale="14" diffuseConstant="1.05"
                         lighting-color="#fff1d6" result="diffuse">
        <feDistantLight azimuth="135" elevation="55"/>
      </feDiffuseLighting>
      <feComposite in="diffuse" in2="SourceGraphic" operator="arithmetic"
                   k1="1" k2="0" k3="0" k4="0"/>
    </filter>

    <!-- Glossy: diffuse + tight specular catchlight. Used for the
         eye spheres, iris, and the bauble — the wet/shiny parts of
         the reference. -->
    <filter id="shade-glossy">
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

  <!-- Big yellow lightning bolt BEHIND the body — the character's
       signature "wielded charge" effect. Larger and more dramatic
       than the previous version, with a dark cartoon outline from a
       same-shape darker path drawn underneath. -->
  <path d="M 258 65 L 195 130 L 250 160 L 200 240 L 320 200 L 270 140 L 268 125 L 318 75 L 285 60 Z"
        fill="#3a2418"/>
  <path d="M 258 70 L 202 132 L 252 160 L 208 232 L 312 198 L 266 142 L 264 128 L 312 80 L 285 67 Z"
        fill="#f5d33a"/>

  <!-- Cape: a flowing flag-shaped 2D <path> on the lower-left, drawn
       behind the body. Anchor at upper-right, curved bulge in the
       middle, tail point at the lower-left — closer to a flowing
       cloak silhouette than the previous round blob. -->
  <path d="M 115 275 C 125 310, 110 350, 75 370 L 45 355 C 50 325, 60 295, 80 280 Z"
        fill="#c8242c" stroke="#3a2418" stroke-width="3" stroke-linejoin="round"/>

  <!-- Hammer head outline + cube. 2D <rect> drawn first as a thin
       dark rim extending slightly outside the cube's silhouette. -->
  <rect x="94" y="44" width="72" height="72" fill="#3a2418"/>
  <cube cx="130" cy="80" cz="-45" size="68"
        fill="#d8d8d8" filter="url(#shade-soft)"/>

  <!-- Hammer pommel / grip: a darker gray cube on TOP of the head. -->
  <rect x="120" y="28" width="20" height="20" fill="#3a2418"/>
  <cube cx="130" cy="38" cz="-40" size="18"
        fill="#a8a8a8" filter="url(#shade-soft)"/>

  <!-- Small bolt decal on the hammer's face -->
  <path d="M 138 56 L 150 56 L 142 78 L 153 78 L 122 110 L 134 84 L 122 84 Z"
        fill="#f5d33a"/>

  <!-- Body outline: dark 2D ellipse slightly larger than the body
       ellipsoid's screen silhouette, drawn before the body so just a
       thin dark rim peeks out around the edge — cartoon outline. -->
  <ellipse cx="220" cy="270" rx="106" ry="82" fill="#3a2418"/>

  <!-- Body bulb: smaller and lower, leaving room for the big bolt and
       the bigger hammer on the upper-left. -->
  <ellipsoid cx="220" cy="270" cz="10" rx="102" ry="78" rz="78"
             fill="#f5a87a" filter="url(#shade-soft)"/>

  <!-- Bottom legs / pincer feet — outline + filled -->
  <ellipse cx="190" cy="340" rx="13" ry="10" fill="#3a2418"/>
  <ellipsoid cx="190" cy="340" cz="20" rx="10" ry="7" rz="7"
             fill="#c8242c" filter="url(#shade-soft)"/>
  <ellipse cx="250" cy="340" rx="13" ry="10" fill="#3a2418"/>
  <ellipsoid cx="250" cy="340" cz="20" rx="10" ry="7" rz="7"
             fill="#c8242c" filter="url(#shade-soft)"/>

  <!-- Left arm + pincer claw, each with a dark outline behind. -->
  <ellipse cx="125" cy="275" rx="29" ry="25" fill="#3a2418"/>
  <ellipsoid cx="125" cy="275" cz="25" rx="26" ry="22" rz="22"
             fill="#f5a87a" filter="url(#shade-soft)"/>
  <ellipse cx="92" cy="290" rx="22" ry="15" fill="#3a2418"/>
  <ellipsoid cx="92" cy="290" cz="30" rx="19" ry="12" rz="12"
             fill="#f5a87a" filter="url(#shade-soft)"/>
  <ellipse cx="88" cy="312" rx="20" ry="15" fill="#3a2418"/>
  <ellipsoid cx="88" cy="312" cz="30" rx="17" ry="12" rz="12"
             fill="#f5a87a" filter="url(#shade-soft)"/>

  <!-- Right arm + pincer claw, mirrored, with outlines. -->
  <ellipse cx="315" cy="275" rx="29" ry="25" fill="#3a2418"/>
  <ellipsoid cx="315" cy="275" cz="25" rx="26" ry="22" rz="22"
             fill="#f5a87a" filter="url(#shade-soft)"/>
  <ellipse cx="352" cy="280" rx="22" ry="15" fill="#3a2418"/>
  <ellipsoid cx="352" cy="280" cz="35" rx="19" ry="12" rz="12"
             fill="#f5a87a" filter="url(#shade-soft)"/>
  <ellipse cx="354" cy="310" rx="22" ry="15" fill="#3a2418"/>
  <ellipsoid cx="354" cy="310" cz="35" rx="19" ry="12" rz="12"
             fill="#f5a87a" filter="url(#shade-soft)"/>

  <!-- White bauble cradled in the right pincer, with outline. -->
  <ellipse cx="368" cy="297" rx="17" ry="17" fill="#3a2418"/>
  <ellipsoid cx="368" cy="297" cz="55" rx="14" ry="14" rz="13"
             fill="#ffffff" filter="url(#shade-glossy)"/>

  <!-- Eyes: oval sclera with dark outline, dominant brown iris (flat-
       disc cylinder), primary + secondary catchlights. -->
  <ellipse cx="185" cy="232" rx="45" ry="51" fill="#3a2418"/>
  <ellipsoid cx="185" cy="232" cz="70" rx="42" ry="48" rz="42"
             fill="#ffffff" filter="url(#shade-glossy)"/>
  <ellipse cx="265" cy="232" rx="45" ry="51" fill="#3a2418"/>
  <ellipsoid cx="265" cy="232" cz="70" rx="42" ry="48" rz="42"
             fill="#ffffff" filter="url(#shade-glossy)"/>

  <cylinder cx="178" cy="247" cz="115" rx="27" ry="32" depth="14"
            fill="#3a2418" filter="url(#shade-glossy)"/>
  <cylinder cx="270" cy="247" cz="115" rx="27" ry="32" depth="14"
            fill="#3a2418" filter="url(#shade-glossy)"/>

  <!-- Primary catchlights (upper-left of each iris) -->
  <ellipsoid cx="170" cy="234" cz="140" rx="8" ry="10" rz="6"
             fill="#ffffff"/>
  <ellipsoid cx="262" cy="234" cz="140" rx="8" ry="10" rz="6"
             fill="#ffffff"/>
  <!-- Secondary catchlights (smaller, lower-right of each iris) -->
  <ellipsoid cx="186" cy="262" cz="140" rx="3" ry="3" rz="2"
             fill="#ffffff"/>
  <ellipsoid cx="278" cy="262" cz="140" rx="3" ry="3" rz="2"
             fill="#ffffff"/>

  <!-- Mouth outline + smile puck. The dark outline behind serves
       double duty as both an outline AND deepens the mouth color. -->
  <ellipse cx="225" cy="305" rx="42" ry="15" fill="#3a2418"/>
  <cylinder cx="225" cy="305" cz="115" rx="38" ry="12" depth="6"
            fill="#1f1209" filter="url(#shade-soft)"/>
  <!-- Tooth: small white block on the left of the mouth, with outline. -->
  <rect x="200" y="298" width="12" height="13" fill="#3a2418"/>
  <cube cx="206" cy="304" cz="125" size="10"
        fill="#ffffff" filter="url(#shade-soft)"/>
</svg>"##;
