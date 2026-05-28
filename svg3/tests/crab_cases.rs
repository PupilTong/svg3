//! SVG fixtures for the 3D crab snapshot tests.
//!
//! The character is built purely from svg3's three SPEC §5 primitives
//! (`<ellipsoid>`, `<cube>`, `<surface>`). All "3D" shading is faked
//! with one shared SVG filter chain that turns the flat silhouette's
//! alpha into a height field, then runs `feDiffuseLighting` +
//! `feSpecularLighting` over it — no scene-lighting feature is
//! required, per the agreed approach.

#![allow(dead_code)]

/// Width for [`CRAB_2D_REFERENCE_SVG`] — matches the 2D asset's
/// `viewBox` X extent. svg3 does not yet honor `viewBox` /
/// `preserveAspectRatio`, so the render target needs to be large
/// enough to contain the source paths' native coordinates or the
/// image clips to a corner of the canvas.
pub const CRAB_2D_REFERENCE_WIDTH: u32 = 2500;
/// Height for [`CRAB_2D_REFERENCE_SVG`].
pub const CRAB_2D_REFERENCE_HEIGHT: u32 = 2345;

/// The 2D reference asset the 3D fixtures are loosely modelled after,
/// loaded from `tests/fixtures/crab_2d.svg`. Snapshotting this through
/// svg3 documents how the project's own 2D renderer handles a real-
/// world stylised SVG (paths + linear gradients + clip-path) — a
/// baseline next to the `crab-full` 3D render.
pub const CRAB_2D_REFERENCE_SVG: &str = include_str!("fixtures/crab_2d.svg");

/// Render size (square) for [`SHADE_PROBE_SVG`].
pub const SHADE_PROBE_SIZE: u32 = 200;

/// The minimal probe: one `<ellipsoid>` with the shade filter applied,
/// validating that the filter pipeline produces fake-3D shading on a
/// 3D primitive's silhouette. This is the "smallest unit of evidence"
/// for the approach.
pub const SHADE_PROBE_SVG: &str = r##"<svg extension="pupiltong" width="200" height="200" xmlns="http://www.w3.org/2000/svg">
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
pub const CRAB_FACE_SVG: &str = r##"<svg extension="pupiltong" width="300" height="300" xmlns="http://www.w3.org/2000/svg">
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

/// The full character — re-authored from scratch against the 2D
/// reference asset. Composition matches the previous fixture closely
/// (peach body bulb low-centre, big eyes with brown radial-gradient
/// irises, hammer floating above-left, lightning bolt above-right,
/// red cape on the lower-left, white bauble cradled in the right
/// pincer, single tooth) but the geometry has been re-derived and
/// the texture/gradient ids cleaned up.
///
/// Each 3D primitive's `fill` resolves to a nested-`<svg>` paint
/// server (§6.1.1) sitting inside `<defs>`. Most of those nested
/// SVGs are a thin wrapper around one rect filled by a top-level
/// `<linearGradient>` / `<radialGradient>`, but the **hammer head**
/// and **mouth** textures are multi-element — `t-hammer` paints a
/// gray panel + a yellow lightning-bolt decal directly onto the cube
/// face, and `t-mouth` paints a layered cavity (warm lip ring →
/// deep dark cavity → faint tongue tint) so the cylinder cap shows
/// real interior depth.
///
/// The gradients themselves live at the outer `<defs>` level —
/// `collect_paint_definitions` stops descending at every non-root
/// `<svg>`, so a gradient nested INSIDE a paint server `<svg>` would
/// be unreachable when that subtree gets rasterized into its texture
/// layer.
pub const CRAB_FULL_SVG: &str = r##"<svg extension="pupiltong" width="400" height="400" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <!-- Matte shading filter: pure diffuse, no specular catchlight.
         Used for the body, arms, legs, hammer, and mouth interior. -->
    <filter id="matte">
      <feGaussianBlur in="SourceAlpha" stdDeviation="5" result="bump"/>
      <feDiffuseLighting in="bump" surfaceScale="14" diffuseConstant="1.05"
                         lighting-color="#fff1d6" result="diff">
        <feDistantLight azimuth="130" elevation="55"/>
      </feDiffuseLighting>
      <feComposite in="diff" in2="SourceGraphic" operator="arithmetic"
                   k1="1" k2="0" k3="0" k4="0"/>
    </filter>

    <!-- Glossy shading filter: diffuse + tight specular catchlight.
         Used for the wet-looking eye spheres, iris discs, and the
         bauble in the right pincer. -->
    <filter id="gloss">
      <feGaussianBlur in="SourceAlpha" stdDeviation="5" result="bump"/>
      <feDiffuseLighting in="bump" surfaceScale="14" diffuseConstant="1"
                         lighting-color="#fff1d6" result="diff">
        <feDistantLight azimuth="130" elevation="55"/>
      </feDiffuseLighting>
      <feComposite in="diff" in2="SourceGraphic" operator="arithmetic"
                   k1="1" k2="0" k3="0" k4="0" result="shaded"/>
      <feSpecularLighting in="bump" surfaceScale="14" specularConstant="0.95"
                          specularExponent="20" lighting-color="#ffffff"
                          result="spec">
        <feDistantLight azimuth="130" elevation="55"/>
      </feSpecularLighting>
      <feComposite in="spec" in2="SourceGraphic" operator="in"
                   result="spec-in"/>
      <feComposite in="spec-in" in2="shaded" operator="arithmetic"
                   k1="0" k2="1" k3="1" k4="0"/>
    </filter>

    <!-- Outer-level gradients (see docstring for why they live here
         rather than inside the nested-SVG paint server wrappers). -->

    <linearGradient id="g-body" x1="20" y1="20" x2="80" y2="100"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFBC9C"/>
      <stop offset="0.7" stop-color="#FFBC9C"/>
      <stop offset="1" stop-color="#FFA67A"/>
    </linearGradient>

    <linearGradient id="g-arm" x1="10" y1="10" x2="90" y2="90"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FBC38C"/>
      <stop offset="0.3" stop-color="#F19675"/>
      <stop offset="0.7" stop-color="#F0876A"/>
      <stop offset="1" stop-color="#ED8870"/>
    </linearGradient>

    <linearGradient id="g-foot" x1="0" y1="0" x2="100" y2="100"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FF5A5A"/>
      <stop offset="1" stop-color="#9D3737"/>
    </linearGradient>

    <radialGradient id="g-iris" cx="50" cy="50" r="50"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#1f1209"/>
      <stop offset="0.4" stop-color="#3a2418"/>
      <stop offset="0.85" stop-color="#564341"/>
      <stop offset="1" stop-color="#845750"/>
    </radialGradient>

    <radialGradient id="g-eye" cx="50" cy="48" r="55"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#ffffff"/>
      <stop offset="0.75" stop-color="#fbeee8"/>
      <stop offset="1" stop-color="#f0d8cc"/>
    </radialGradient>

    <radialGradient id="g-bauble" cx="40" cy="35" r="55"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#ffffff"/>
      <stop offset="0.6" stop-color="#FFF9CE"/>
      <stop offset="1" stop-color="#f0e7b8"/>
    </radialGradient>

    <linearGradient id="g-hammer" x1="0" y1="0" x2="100" y2="100"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#D2D2D2"/>
      <stop offset="0.55" stop-color="#B6B6B6"/>
      <stop offset="1" stop-color="#7D7C7D"/>
    </linearGradient>

    <linearGradient id="g-grip" x1="0" y1="0" x2="100" y2="100"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#A8A8A8"/>
      <stop offset="1" stop-color="#7D7C7D"/>
    </linearGradient>

    <linearGradient id="g-mouth" x1="0" y1="0" x2="0" y2="100"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#1f1209"/>
      <stop offset="1" stop-color="#3a2418"/>
    </linearGradient>

    <!-- Simple texture paint servers — each wraps one rect filled by
         the matching gradient. -->
    <svg id="t-body" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-body)"/>
    </svg>
    <svg id="t-arm" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-arm)"/>
    </svg>
    <svg id="t-foot" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-foot)"/>
    </svg>
    <svg id="t-iris" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-iris)"/>
    </svg>
    <svg id="t-eye" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-eye)"/>
    </svg>
    <svg id="t-bauble" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-bauble)"/>
    </svg>
    <svg id="t-grip" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-grip)"/>
    </svg>

    <!-- Multi-element textures. The user asked for the hammer's skin
         and the crab's mouth to be authored from rich nested SVG
         content, not single-rect gradient swatches. With cube-map="same"
         (default), the cube face shows the whole texture upright, so
         the bolt decal lands on every visible face of the hammer head.
         The cylinder cap's polar-disk UV (§6.1.1) puts the centre of
         `t-mouth` at the centre of the mouth, so a deep-dark inner
         circle reads as cavity depth and the warmer rim reads as
         lip-line. -->
    <svg id="t-hammer" viewBox="0 0 100 100">
      <!-- Gray panel base -->
      <rect width="100" height="100" fill="url(#g-hammer)"/>
      <!-- Darker rim around the panel -->
      <rect x="2" y="2" width="96" height="96" fill="none"
            stroke="#7D7C7D" stroke-width="3"/>
      <!-- Yellow lightning-bolt decal in the centre of the face -->
      <path d="M 56 18 L 70 18 L 60 42 L 78 42 L 28 84 L 50 52 L 32 52 Z"
            fill="#FFE768" stroke="#3a2418" stroke-width="2"
            stroke-linejoin="round"/>
    </svg>

    <svg id="t-mouth" viewBox="0 0 100 100">
      <!-- Lip line: medium dark brown rim -->
      <rect width="100" height="100" fill="url(#g-mouth)"/>
      <!-- Deep cavity: very dark centre -->
      <circle cx="50" cy="50" r="36" fill="#1f1209"/>
      <!-- Tongue suggestion: faint red below centre -->
      <ellipse cx="50" cy="64" rx="22" ry="9" fill="#5a2018" opacity="0.6"/>
    </svg>
  </defs>

  <!-- ============== BACKGROUND LAYER ============== -->

  <!-- Yellow lightning bolt (upper-right, behind the body). -->
  <path d="M 260 60 L 200 130 L 252 158 L 205 240 L 322 200 L 272 142 L 270 126 L 320 76 L 285 58 Z"
        fill="#3a2418"/>
  <path d="M 260 66 L 207 132 L 254 158 L 213 232 L 314 198 L 268 144 L 266 130 L 314 80 L 285 65 Z"
        fill="#FFE768"/>

  <!-- Red cape (lower-left, peeking from behind the body). -->
  <path d="M 118 280 C 128 318, 112 358, 78 376 L 44 358 C 50 326, 62 296, 84 282 Z"
        fill="#c8242c" stroke="#3a2418" stroke-width="3" stroke-linejoin="round"/>

  <!-- ============== HAMMER (upper-left, above the body) ============== -->

  <!-- Hammer head: cube with a slightly larger dark outline rect
       behind. The yellow bolt decal lives inside the `t-hammer`
       texture, so it's painted directly onto the cube faces — no
       separate 2D `<path>` is drawn on top. -->
  <rect x="92" y="42" width="74" height="74" fill="#3a2418"/>
  <cube cx="129" cy="79" cz="20" size="68"
        fill="url(#t-hammer)" filter="url(#matte)"/>

  <!-- Hammer grip: smaller, darker cube sitting on top of the head. -->
  <rect x="120" y="26" width="20" height="22" fill="#3a2418"/>
  <cube cx="130" cy="37" cz="25" size="18"
        fill="url(#t-grip)" filter="url(#matte)"/>

  <!-- ============== BODY + FEET ============== -->

  <ellipse cx="215" cy="270" rx="108" ry="84" fill="#3a2418"/>
  <ellipsoid cx="215" cy="270" cz="10" rx="104" ry="80" rz="80"
             fill="url(#t-body)" filter="url(#matte)"/>

  <!-- Feet — small red boots peeking under the body. -->
  <ellipse cx="184" cy="342" rx="14" ry="11" fill="#3a2418"/>
  <ellipsoid cx="184" cy="342" cz="20" rx="11" ry="8" rz="8"
             fill="url(#t-foot)" filter="url(#matte)"/>
  <ellipse cx="246" cy="342" rx="14" ry="11" fill="#3a2418"/>
  <ellipsoid cx="246" cy="342" cz="20" rx="11" ry="8" rz="8"
             fill="url(#t-foot)" filter="url(#matte)"/>

  <!-- ============== ARMS + PINCER CLAWS ============== -->

  <ellipse cx="122" cy="278" rx="30" ry="26" fill="#3a2418"/>
  <ellipsoid cx="122" cy="278" cz="25" rx="27" ry="23" rz="23"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="88" cy="292" rx="23" ry="16" fill="#3a2418"/>
  <ellipsoid cx="88" cy="292" cz="30" rx="20" ry="13" rz="13"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="84" cy="316" rx="21" ry="16" fill="#3a2418"/>
  <ellipsoid cx="84" cy="316" cz="30" rx="18" ry="13" rz="13"
             fill="url(#t-arm)" filter="url(#matte)"/>

  <ellipse cx="312" cy="278" rx="30" ry="26" fill="#3a2418"/>
  <ellipsoid cx="312" cy="278" cz="25" rx="27" ry="23" rz="23"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="350" cy="282" rx="23" ry="16" fill="#3a2418"/>
  <ellipsoid cx="350" cy="282" cz="35" rx="20" ry="13" rz="13"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="354" cy="312" rx="23" ry="16" fill="#3a2418"/>
  <ellipsoid cx="354" cy="312" cz="35" rx="20" ry="13" rz="13"
             fill="url(#t-arm)" filter="url(#matte)"/>

  <!-- White bauble cradled in the right pincer (glossy, with outline). -->
  <ellipse cx="368" cy="298" rx="18" ry="18" fill="#3a2418"/>
  <ellipsoid cx="368" cy="298" cz="55" rx="15" ry="15" rz="14"
             fill="url(#t-bauble)" filter="url(#gloss)"/>

  <!-- ============== FACE ============== -->

  <ellipse cx="180" cy="232" rx="46" ry="52" fill="#3a2418"/>
  <ellipsoid cx="180" cy="232" cz="70" rx="43" ry="49" rz="43"
             fill="url(#t-eye)" filter="url(#gloss)"/>
  <cylinder cx="172" cy="248" cz="115" rx="28" ry="33" depth="14"
            fill="url(#t-iris)" filter="url(#gloss)"/>
  <ellipsoid cx="164" cy="234" cz="140" rx="9" ry="11" rz="6"
             fill="#ffffff"/>
  <ellipsoid cx="180" cy="262" cz="140" rx="3" ry="3" rz="2"
             fill="#ffffff"/>

  <ellipse cx="262" cy="232" rx="46" ry="52" fill="#3a2418"/>
  <ellipsoid cx="262" cy="232" cz="70" rx="43" ry="49" rz="43"
             fill="url(#t-eye)" filter="url(#gloss)"/>
  <cylinder cx="270" cy="248" cz="115" rx="28" ry="33" depth="14"
            fill="url(#t-iris)" filter="url(#gloss)"/>
  <ellipsoid cx="258" cy="234" cz="140" rx="9" ry="11" rz="6"
             fill="#ffffff"/>
  <ellipsoid cx="278" cy="262" cz="140" rx="3" ry="3" rz="2"
             fill="#ffffff"/>

  <!-- Mouth: dark cylinder cavity with a small white cube tooth. -->
  <ellipse cx="220" cy="307" rx="44" ry="16" fill="#3a2418"/>
  <cylinder cx="220" cy="307" cz="115" rx="40" ry="13" depth="6"
            fill="url(#t-mouth)" filter="url(#matte)"/>
  <rect x="194" y="298" width="14" height="14" fill="#3a2418"/>
  <cube cx="201" cy="304" cz="125" size="11"
        fill="#ffffff" filter="url(#matte)"/>
</svg>"##;
