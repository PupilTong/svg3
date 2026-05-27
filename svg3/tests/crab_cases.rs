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

  <!-- Cape: lower-left, two stacked red blobs flowing out behind. -->
  <ellipsoid cx="110" cy="305" cz="0" rx="48" ry="60" rz="13"
             fill="#c8242c" filter="url(#shade-soft)"/>
  <ellipsoid cx="95" cy="265" cz="0" rx="32" ry="40" rz="9"
             fill="#a01a22" filter="url(#shade-soft)"/>

  <!-- Hammer head: gray cube floating above-left of the body. Pushed
       back so its front face sits at z = -1, behind any 2D paths drawn
       on top of it. -->
  <cube cx="180" cy="70" cz="-30" size="58"
        fill="#d8d8d8" filter="url(#shade-soft)"/>

  <!-- Lightning-bolt decal: a 2D zigzag <path> on top of the hammer.
       2D content's per-shape forward Z bias places it at z ≈ +0.X,
       cleanly in front of the cube's front face at z = -1, so the
       bolt reads as a yellow zigzag on the gray hammer face. -->
  <path d="M 188 45 L 200 45 L 192 65 L 203 65 L 174 96 L 184 72 L 173 72 Z"
        fill="#f5d33a"/>

  <!-- Body bulb: wide squat peach. Matte filter so it reads as the
       soft gradient peach of the 2D reference, not glossy ceramic. -->
  <ellipsoid cx="210" cy="245" cz="10" rx="118" ry="88" rz="85"
             fill="#f5a87a" filter="url(#shade-soft)"/>

  <!-- Bottom legs / pincer feet -->
  <ellipsoid cx="175" cy="325" cz="20" rx="11" ry="8" rz="8"
             fill="#c8242c" filter="url(#shade-soft)"/>
  <ellipsoid cx="245" cy="325" cz="20" rx="11" ry="8" rz="8"
             fill="#c8242c" filter="url(#shade-soft)"/>

  <!-- Left arm + pincer claw (upper + lower halves of the split pincer) -->
  <ellipsoid cx="108" cy="248" cz="25" rx="30" ry="24" rz="24"
             fill="#f5a87a" filter="url(#shade-soft)"/>
  <ellipsoid cx="72" cy="265" cz="30" rx="22" ry="13" rz="13"
             fill="#f5a87a" filter="url(#shade-soft)"/>
  <ellipsoid cx="68" cy="285" cz="30" rx="20" ry="13" rz="13"
             fill="#f5a87a" filter="url(#shade-soft)"/>

  <!-- Right arm + pincer claw, mirrored -->
  <ellipsoid cx="320" cy="245" cz="25" rx="30" ry="24" rz="24"
             fill="#f5a87a" filter="url(#shade-soft)"/>
  <ellipsoid cx="360" cy="258" cz="35" rx="22" ry="13" rz="13"
             fill="#f5a87a" filter="url(#shade-soft)"/>
  <ellipsoid cx="362" cy="288" cz="35" rx="22" ry="13" rz="13"
             fill="#f5a87a" filter="url(#shade-soft)"/>

  <!-- White bauble cradled in the right pincer, sitting in front -->
  <ellipsoid cx="375" cy="275" cz="55" rx="15" ry="15" rz="14"
             fill="#ffffff" filter="url(#shade-glossy)"/>

  <!-- Eyes: oval sclera, dominant brown iris, primary + secondary
       catchlights. Glossy filter so the iris has the wet catchlight
       look. Pushed close enough that the two eyes touch in screen
       space — characteristic of the reference's "fused" look. -->
  <ellipsoid cx="172" cy="210" cz="70" rx="44" ry="50" rz="44"
             fill="#ffffff" filter="url(#shade-glossy)"/>
  <ellipsoid cx="252" cy="210" cz="70" rx="44" ry="50" rz="44"
             fill="#ffffff" filter="url(#shade-glossy)"/>

  <!-- Iris: cylinder for a flatter disc look (matches the 2D's
       essentially-flat iris fill) rather than a sphere bulge. -->
  <cylinder cx="163" cy="225" cz="115" rx="29" ry="34" depth="14"
            fill="#3a2418" filter="url(#shade-glossy)"/>
  <cylinder cx="261" cy="225" cz="115" rx="29" ry="34" depth="14"
            fill="#3a2418" filter="url(#shade-glossy)"/>

  <!-- Primary catchlights (upper-left of each iris) -->
  <ellipsoid cx="154" cy="213" cz="140" rx="8" ry="10" rz="6"
             fill="#ffffff"/>
  <ellipsoid cx="252" cy="213" cz="140" rx="8" ry="10" rz="6"
             fill="#ffffff"/>
  <!-- Secondary catchlights (smaller, lower-right of each iris) -->
  <ellipsoid cx="172" cy="242" cz="140" rx="3" ry="3" rz="2"
             fill="#ffffff"/>
  <ellipsoid cx="270" cy="242" cz="140" rx="3" ry="3" rz="2"
             fill="#ffffff"/>

  <!-- Mouth: wide thin dark "smile puck" pushed in front of the body's
       front face at this screen position (body front ≈ z = 88). -->
  <cylinder cx="210" cy="278" cz="115" rx="42" ry="13" depth="6"
            fill="#1f1209" filter="url(#shade-soft)"/>
  <!-- Tooth: small white block on the left of the mouth -->
  <cube cx="188" cy="276" cz="125" size="11"
        fill="#ffffff" filter="url(#shade-soft)"/>
</svg>"##;
