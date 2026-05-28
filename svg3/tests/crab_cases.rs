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

/// The full character — re-authored against `tests/fixtures/crab_2d.svg`
/// with the deliberate goal of *filling the canvas*. The previous pass
/// of this fixture rendered as a small icon-sized character; here the
/// body bulb, eyes, hammer, and lightning bolt are all sized up so the
/// crab reads as a proper character rather than a battery decal.
///
/// Layout (left → right, top → bottom):
///
/// - **Hammer** at the top-left, sized as a wide rectangular brick
///   (Mjolnir-shaped: `width=110 height=80 depth=68`) with a small
///   pommel cube above it; the gray panel + yellow lightning-bolt
///   decal both live inside the `t-hammer` nested-`<svg>` paint
///   server (§6.1.1) so the bolt is painted directly onto the cube
///   faces.
/// - **Lightning bolt** behind the upper-right, a jagged dark-outlined
///   yellow 2D path the eye/head reads in front of.
/// - **Red cape** as a flowing flag-shaped 2D path on the lower-left,
///   peeking out from behind the body.
/// - **Body bulb** filling the lower-middle (`rx=130 ry=95`), painted
///   from `t-body` (mirrors crab_2d.svg's gradient `#b`).
/// - **Huge eyes** dominating the upper face — sclera ellipsoids with
///   `rx=56 ry=64`, brown iris cylinders covering most of the eye, and
///   two catchlights per eye: a big upper-left bright spot and a small
///   secondary lower-right glint.
/// - **Wide mouth** with a single white tooth cube on the left,
///   painted from `t-mouth` whose layered cavity (rim → deep dark
///   centre → tongue tint) gives the polar-disk-mapped cap real
///   interior depth.
/// - **Pincer arms** reaching out to the sides; **bauble** cradled in
///   the right pincer; **feet** as two small red boots peeking under
///   the bottom of the body.
///
/// Each 3D primitive's `fill` resolves to a nested-`<svg>` paint
/// server. Most are a thin wrapper around one rect filled by a
/// top-level `<linearGradient>` / `<radialGradient>` mirroring the
/// crab_2d.svg palette (`#b`, `#s`, `#j`, `#y`, etc.); the **hammer**
/// and **mouth** are multi-element (per the user's request, so the
/// bolt decal and the mouth cavity come through the texture
/// pipeline, not as 2D overlays). The gradients themselves must live
/// at the outer `<defs>` level — `collect_paint_definitions` stops
/// descending at every non-root `<svg>`, so a gradient nested inside
/// a paint-server `<svg>` would be unreachable when that subtree
/// gets rasterized into its texture layer.
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
         rather than inside the nested-SVG paint-server wrappers). -->

    <linearGradient id="g-body" x1="15" y1="15" x2="85" y2="100"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFBC9C"/>
      <stop offset="0.55" stop-color="#FFBC9C"/>
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

    <radialGradient id="g-iris" cx="50" cy="50" r="55"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#1f1209"/>
      <stop offset="0.35" stop-color="#3a2418"/>
      <stop offset="0.8" stop-color="#564341"/>
      <stop offset="1" stop-color="#845750"/>
    </radialGradient>

    <radialGradient id="g-eye" cx="50" cy="48" r="55"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#ffffff"/>
      <stop offset="0.78" stop-color="#fbeee8"/>
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
      <stop offset="0" stop-color="#D6D6D6"/>
      <stop offset="0.5" stop-color="#B6B6B6"/>
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

    <!-- Simple gradient-swatch paint servers. -->
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

    <!-- Multi-element textures (per user request: "the crab mouth and
         hammer's skin should use nested svg elements").

         `t-hammer`: gray gradient panel + yellow lightning-bolt decal
         with its own dark cartoon outline (stroke). With cube-map="same"
         (default), every face of the hammer cube shows the same panel,
         and the bolt appears on whichever face the camera frames.

         `t-mouth`: warm-dark lip ring (g-mouth) → deep-dark cavity
         circle (#1f1209) → faint red tongue ellipse. The cylinder
         cap's polar-disk UV (§6.1.1) puts the texture centre at the
         centre of the mouth, so the radial darkness reads as cavity
         depth and the warmer rim reads as lip-line. -->
    <svg id="t-hammer" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-hammer)"/>
      <path d="M 56 14 L 72 14 L 60 42 L 80 42 L 28 86 L 50 52 L 30 52 Z"
            fill="#FFE768" stroke="#3a2418" stroke-width="3"
            stroke-linejoin="round"/>
    </svg>

    <svg id="t-mouth" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-mouth)"/>
      <circle cx="50" cy="50" r="36" fill="#1f1209"/>
      <ellipse cx="50" cy="64" rx="22" ry="9" fill="#5a2018" opacity="0.6"/>
    </svg>
  </defs>

  <!-- ============== BACKGROUND LAYER ============== -->

  <!-- Big yellow lightning bolt behind the body, upper-right. Dark
       outline path drawn first; yellow main path slightly inset on
       top. -->
  <path d="M 280 38 L 215 130 L 275 165 L 220 250 L 345 200 L 290 142 L 288 128 L 343 73 L 308 38 Z"
        fill="#3a2418"/>
  <path d="M 280 46 L 222 132 L 278 165 L 228 240 L 335 198 L 286 144 L 284 132 L 335 78 L 308 46 Z"
        fill="#FFE768"/>

  <!-- Red flowing cape: a flag-silhouette 2D path on the lower-left,
       anchored at the body's left shoulder and curling out to the
       lower-left corner. Drawn before the body so the body bulb
       covers the cape's upper-right; only the flowing outer half
       reads in the final composition. -->
  <path d="M 80 220 C 105 270, 88 360, 5 392 L 0 320 C 5 280, 25 230, 60 215 Z"
        fill="#c8242c" stroke="#3a2418" stroke-width="4" stroke-linejoin="round"/>

  <!-- ============== HAMMER (top-left, prominent) ============== -->

  <!-- Hammer head: wide rectangular cube (width > height > depth so
       it reads as a brick-shaped Mjolnir hammer face), with a thick
       dark outline rect behind for the cartoon rim. The yellow bolt
       decal lives inside the `t-hammer` texture, so it's painted
       directly onto the cube face — no separate 2D <path> on top. -->
  <rect x="60" y="38" width="120" height="92" fill="#3a2418"/>
  <cube cx="120" cy="84" cz="25" width="110" height="80" depth="68"
        fill="url(#t-hammer)" filter="url(#matte)"/>

  <!-- Hammer pommel / grip: a small darker cube sitting on top of
       the head, giving the hammer a proper handle shape. -->
  <rect x="106" y="18" width="28" height="24" fill="#3a2418"/>
  <cube cx="120" cy="32" cz="30" width="24" height="22" depth="22"
        fill="url(#t-grip)" filter="url(#matte)"/>

  <!-- ============== BODY + FEET ============== -->

  <!-- Body cartoon outline (2D <ellipse> slightly larger than the
       ellipsoid's screen silhouette) + body bulb. Wider and lower
       than the previous fixture so the crab fills the canvas. -->
  <ellipse cx="212" cy="268" rx="135" ry="100" fill="#3a2418"/>
  <ellipsoid cx="212" cy="268" cz="10" rx="130" ry="95" rz="95"
             fill="url(#t-body)" filter="url(#matte)"/>

  <!-- Feet — two small red boots peeking under the body. -->
  <ellipse cx="175" cy="358" rx="17" ry="13" fill="#3a2418"/>
  <ellipsoid cx="175" cy="358" cz="20" rx="14" ry="10" rz="10"
             fill="url(#t-foot)" filter="url(#matte)"/>
  <ellipse cx="258" cy="358" rx="17" ry="13" fill="#3a2418"/>
  <ellipsoid cx="258" cy="358" cz="20" rx="14" ry="10" rz="10"
             fill="url(#t-foot)" filter="url(#matte)"/>

  <!-- ============== ARMS + PINCER CLAWS ============== -->

  <!-- Left arm: shoulder + split pincer claw made of two ellipsoids. -->
  <ellipse cx="100" cy="292" rx="36" ry="32" fill="#3a2418"/>
  <ellipsoid cx="100" cy="292" cz="25" rx="33" ry="29" rz="29"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="60" cy="298" rx="26" ry="18" fill="#3a2418"/>
  <ellipsoid cx="60" cy="298" cz="30" rx="23" ry="15" rz="15"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="54" cy="328" rx="24" ry="18" fill="#3a2418"/>
  <ellipsoid cx="54" cy="328" cz="30" rx="21" ry="15" rz="15"
             fill="url(#t-arm)" filter="url(#matte)"/>

  <!-- Right arm: shoulder + split pincer, mirrored. -->
  <ellipse cx="330" cy="292" rx="36" ry="32" fill="#3a2418"/>
  <ellipsoid cx="330" cy="292" cz="25" rx="33" ry="29" rz="29"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="370" cy="295" rx="26" ry="18" fill="#3a2418"/>
  <ellipsoid cx="370" cy="295" cz="35" rx="23" ry="15" rz="15"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="375" cy="325" rx="24" ry="18" fill="#3a2418"/>
  <ellipsoid cx="375" cy="325" cz="35" rx="21" ry="15" rz="15"
             fill="url(#t-arm)" filter="url(#matte)"/>

  <!-- White bauble cradled in the right pincer (glossy, with outline). -->
  <ellipse cx="388" cy="310" rx="20" ry="20" fill="#3a2418"/>
  <ellipsoid cx="388" cy="310" cz="55" rx="17" ry="17" rz="16"
             fill="url(#t-bauble)" filter="url(#gloss)"/>

  <!-- ============== FACE: HUGE EYES ============== -->

  <!-- Left eye: dominant brown iris (cylinder, polar-disk UV) sits
       on a white sclera (ellipsoid). Two catchlights: a big bright
       primary up-and-left of the pupil, a small secondary
       lower-right glint. The eye is intentionally large so the
       irises read as the focal point of the face. -->
  <ellipse cx="167" cy="225" rx="60" ry="68" fill="#3a2418"/>
  <ellipsoid cx="167" cy="225" cz="70" rx="56" ry="64" rz="56"
             fill="url(#t-eye)" filter="url(#gloss)"/>
  <cylinder cx="158" cy="245" cz="115" rx="40" ry="46" depth="14"
            fill="url(#t-iris)" filter="url(#gloss)"/>
  <ellipsoid cx="140" cy="222" cz="140" rx="15" ry="19" rz="9"
             fill="#ffffff"/>
  <ellipsoid cx="172" cy="268" cz="140" rx="5" ry="5" rz="3"
             fill="#ffffff"/>

  <!-- Right eye: mirrored. -->
  <ellipse cx="280" cy="225" rx="60" ry="68" fill="#3a2418"/>
  <ellipsoid cx="280" cy="225" cz="70" rx="56" ry="64" rz="56"
             fill="url(#t-eye)" filter="url(#gloss)"/>
  <cylinder cx="290" cy="245" cz="115" rx="40" ry="46" depth="14"
            fill="url(#t-iris)" filter="url(#gloss)"/>
  <ellipsoid cx="262" cy="222" cz="140" rx="15" ry="19" rz="9"
             fill="#ffffff"/>
  <ellipsoid cx="296" cy="268" cz="140" rx="5" ry="5" rz="3"
             fill="#ffffff"/>

  <!-- ============== MOUTH + TOOTH ============== -->

  <!-- Wide cartoon mouth: 2D outline ellipse + a shallow cylinder
       painted by the layered `t-mouth` cavity texture. A small white
       cube tooth on the left of the mouth (with its own outline). -->
  <ellipse cx="222" cy="318" rx="52" ry="18" fill="#3a2418"/>
  <cylinder cx="222" cy="318" cz="115" rx="48" ry="15" depth="6"
            fill="url(#t-mouth)" filter="url(#matte)"/>
  <rect x="192" y="312" width="16" height="14" fill="#3a2418"/>
  <cube cx="200" cy="320" cz="125" size="12"
        fill="#ffffff" filter="url(#matte)"/>
</svg>"##;
