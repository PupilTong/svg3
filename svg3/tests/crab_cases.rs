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

/// The full character — re-authored to actually match the 2D
/// reference asset's anatomy (review feedback round 3).
///
/// **What the reviewer flagged in the previous pass:**
///
/// - The two pincers were symmetric and tiny. In the reference one
///   claw is *massive* and nearly the size of the body — it's also
///   where the hammer rests. The other side just has a small claw.
/// - The body was too flat / horizontally stretched. It should read
///   as a chubby, near-spherical bean.
/// - Two tiny red stubs at the bottom are not "legs"; the reference
///   has 3 distinct pointed crab legs on each side.
/// - The hammer floated in the air. It needs a visible handle going
///   down into the giant claw.
/// - The eyes were two separate plastic spheres staring straight
///   ahead. The reference has them as interlocking vertical ovals
///   with pupils gazing up-and-to-the-left.
/// - The open-mouth-with-tooth read as "shocked"; the reference is
///   a subtle off-centre smirk line.
/// - The shading was too plastic / glossy. The reference is soft
///   cel-shading.
///
/// **This pass:**
///
/// - **Giant left claw** (viewer's left = character's right): an
///   ellipsoid roughly as big as the body, painted from `t-body` so
///   the silhouette merges into the body.
/// - **Small right claw** with a pincer tip on the opposite side.
/// - **Hammer** sits on top of the giant claw, connected by a thin
///   dark **handle cube** going down into the claw's centre. The
///   hammer head's nested-`<svg>` paint server still paints the
///   yellow lightning-bolt decal directly onto the cube faces.
/// - **Body** reshaped to be near-spherical (`rx≈ry≈rz`) and plump.
/// - **6 crab legs** (3 on each side) as tall narrow ellipsoids
///   sticking down from under the body.
/// - **Lightning bolt** repositioned to grow up from behind the
///   head, the lower tip rooted near the right eye.
/// - **Cape** repositioned to flow back-and-down behind the giant
///   claw, anchored at the claw's upper edge.
/// - **Eyes** authored as interlocking vertical ovals — the inner
///   edges overlap so the eye silhouettes touch in the middle. The
///   iris cylinders are shifted up-and-left within each eye so the
///   pupils gaze up-left like the reference.
/// - **Smirk mouth**: a single curved stroked path on the body.
///   The cylinder + tooth open-mouth set-up is gone.
/// - **Softened filters**: matte and gloss both have lower
///   `diffuseConstant` / `specularConstant`, more bump blur, and a
///   higher light elevation so the surfaces stop looking like
///   moulded plastic. Eye sclera moved off `gloss` onto `matte` so
///   the eyes don't read as ceramic eyeballs.
///
/// Hammer and mouth textures still use multi-element nested-`<svg>`
/// paint servers (§6.1.1) per the original feedback round.
pub const CRAB_FULL_SVG: &str = r##"<svg extension="pupiltong" width="400" height="400" xmlns="http://www.w3.org/2000/svg">
  <defs>
    <!-- Softened matte filter: bigger bump blur, lower diffuse
         constant, light moved overhead so the body reads as soft
         cel-shading rather than glossy plastic. -->
    <filter id="matte">
      <feGaussianBlur in="SourceAlpha" stdDeviation="8" result="bump"/>
      <feDiffuseLighting in="bump" surfaceScale="10" diffuseConstant="0.95"
                         lighting-color="#fff4dc" result="diff">
        <feDistantLight azimuth="120" elevation="65"/>
      </feDiffuseLighting>
      <feComposite in="diff" in2="SourceGraphic" operator="arithmetic"
                   k1="1" k2="0" k3="0" k4="0"/>
    </filter>

    <!-- Softened gloss filter: same diffuse base as matte, with a
         smaller / tighter specular highlight. Used only on the
         iris + bauble — eye scleras now use `matte` so the eyeballs
         stop looking like ceramic. -->
    <filter id="gloss">
      <feGaussianBlur in="SourceAlpha" stdDeviation="8" result="bump"/>
      <feDiffuseLighting in="bump" surfaceScale="10" diffuseConstant="0.95"
                         lighting-color="#fff4dc" result="diff">
        <feDistantLight azimuth="120" elevation="65"/>
      </feDiffuseLighting>
      <feComposite in="diff" in2="SourceGraphic" operator="arithmetic"
                   k1="1" k2="0" k3="0" k4="0" result="shaded"/>
      <feSpecularLighting in="bump" surfaceScale="10" specularConstant="0.45"
                          specularExponent="30" lighting-color="#ffffff"
                          result="spec">
        <feDistantLight azimuth="120" elevation="65"/>
      </feSpecularLighting>
      <feComposite in="spec" in2="SourceGraphic" operator="in"
                   result="spec-in"/>
      <feComposite in="spec-in" in2="shaded" operator="arithmetic"
                   k1="0" k2="1" k3="1" k4="0"/>
    </filter>

    <!-- Outer-level gradients. Warmer peach palette for the body
         and limbs (`#FFD0B0 → #FF8A6C`) so the gradient stops feel
         more like the reference's airbrushed peach-to-orange. -->

    <linearGradient id="g-body" x1="15" y1="15" x2="85" y2="100"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FFD0B0"/>
      <stop offset="0.5" stop-color="#FFAB8E"/>
      <stop offset="1" stop-color="#FF8A6C"/>
    </linearGradient>

    <linearGradient id="g-arm" x1="10" y1="10" x2="90" y2="90"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#FBC38C"/>
      <stop offset="0.3" stop-color="#F19675"/>
      <stop offset="0.7" stop-color="#F0876A"/>
      <stop offset="1" stop-color="#ED8870"/>
    </linearGradient>

    <radialGradient id="g-iris" cx="50" cy="50" r="55"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#1f1209"/>
      <stop offset="0.35" stop-color="#3a2418"/>
      <stop offset="0.8" stop-color="#564341"/>
      <stop offset="1" stop-color="#845750"/>
    </radialGradient>

    <radialGradient id="g-eye" cx="50" cy="45" r="55"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#ffffff"/>
      <stop offset="0.8" stop-color="#fbeee8"/>
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

    <linearGradient id="g-handle" x1="0" y1="0" x2="100" y2="100"
                    gradientUnits="userSpaceOnUse">
      <stop offset="0" stop-color="#564341"/>
      <stop offset="1" stop-color="#3a2418"/>
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
    <svg id="t-iris" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-iris)"/>
    </svg>
    <svg id="t-eye" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-eye)"/>
    </svg>
    <svg id="t-bauble" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-bauble)"/>
    </svg>
    <svg id="t-handle" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-handle)"/>
    </svg>

    <!-- Multi-element textures (kept from the previous round).

         `t-hammer`: gray gradient panel + yellow lightning-bolt
         decal with its own dark cartoon outline. -->
    <svg id="t-hammer" viewBox="0 0 100 100">
      <rect width="100" height="100" fill="url(#g-hammer)"/>
      <path d="M 56 14 L 72 14 L 60 42 L 80 42 L 28 86 L 50 52 L 30 52 Z"
            fill="#FFE768" stroke="#3a2418" stroke-width="3"
            stroke-linejoin="round"/>
    </svg>
  </defs>

  <!-- ============== BACKGROUND ============== -->

  <!-- Big yellow lightning bolt rooted into the upper-right of the
       head and growing up-and-back. Drawn first so the eyes and
       body cover its lower portion, leaving just the jagged top
       half visible as a stylised crest. -->
  <path d="M 310 30 L 245 130 L 305 130 L 260 220 L 380 130 L 320 130 L 365 30 Z"
        fill="#3a2418"/>
  <path d="M 310 38 L 252 130 L 305 132 L 268 210 L 372 130 L 322 132 L 360 38 Z"
        fill="#FFE768"/>

  <!-- Red cape: a flowing flag silhouette anchored at the upper-back
       of the giant left claw, curling out to the lower-left corner.
       Drawn behind the giant claw so only the outer half shows; the
       visible cape "tail" reads as fabric flowing behind and below
       the character. -->
  <path d="M 130 195 C 135 260, 80 360, 0 398 L 0 280 C 5 235, 35 195, 95 175 Z"
        fill="#c8242c" stroke="#3a2418" stroke-width="4" stroke-linejoin="round"/>

  <!-- ============== GIANT LEFT CLAW + HAMMER ============== -->

  <!-- Giant claw on the viewer's left, nearly as big as the body
       (the asymmetric pincer the reviewer called out). Painted from
       `t-body` so it shares the body's peach palette and the
       silhouette merges with the body's. -->
  <ellipse cx="95" cy="250" rx="92" ry="88" fill="#3a2418"/>
  <ellipsoid cx="95" cy="250" cz="15" rx="88" ry="84" rz="84"
             fill="url(#t-body)" filter="url(#matte)"/>

  <!-- A small dark pincer cleft hinting at the claw's split — a
       thin tilted ellipse in the lower-left of the claw. -->
  <ellipse cx="55" cy="285" rx="22" ry="6" fill="#3a2418"
           transform="rotate(-18 55 285)"/>

  <!-- Hammer handle: a short thick dark stub between the hammer
       head and the claw top. Sized so only a small handle portion
       is visible above the claw; the rest of the handle disappears
       into the claw silhouette. -->
  <rect x="83" y="155" width="30" height="40" fill="#3a2418"/>
  <cube cx="98" cy="178" cz="22" width="26" height="44" depth="22"
        fill="url(#t-handle)" filter="url(#matte)"/>

  <!-- Hammer head: wide Mjolnir-shaped brick sitting just above the
       claw with the handle reaching into it. The yellow bolt decal
       lives inside `t-hammer`, so the face paints itself — no
       separate 2D <path> on top. -->
  <rect x="40" y="98" width="124" height="80" fill="#3a2418"/>
  <cube cx="102" cy="138" cz="28" width="118" height="74" depth="62"
        fill="url(#t-hammer)" filter="url(#matte)"/>

  <!-- ============== BODY ============== -->

  <!-- Body: chubby, near-spherical bulb (rx≈ry≈rz). Sits behind the
       giant claw and to the right; the silhouettes merge into one
       bean. -->
  <ellipse cx="240" cy="270" rx="115" ry="108" fill="#3a2418"/>
  <ellipsoid cx="240" cy="270" cz="10" rx="110" ry="103" rz="103"
             fill="url(#t-body)" filter="url(#matte)"/>

  <!-- ============== CRAB LEGS (3 each side) ============== -->

  <!-- Tall narrow ellipsoids sticking down from under the body so
       the crab actually reads as a crab. Each has a slightly
       larger 2D outline ellipse behind for the cartoon rim. -->
  <ellipse cx="155" cy="370" rx="9" ry="20" fill="#3a2418"/>
  <ellipsoid cx="155" cy="370" cz="15" rx="7" ry="17" rz="7"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="185" cy="378" rx="9" ry="18" fill="#3a2418"/>
  <ellipsoid cx="185" cy="378" cz="15" rx="7" ry="15" rz="7"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="215" cy="383" rx="9" ry="16" fill="#3a2418"/>
  <ellipsoid cx="215" cy="383" cz="15" rx="7" ry="13" rz="7"
             fill="url(#t-arm)" filter="url(#matte)"/>

  <ellipse cx="262" cy="383" rx="9" ry="16" fill="#3a2418"/>
  <ellipsoid cx="262" cy="383" cz="15" rx="7" ry="13" rz="7"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="292" cy="378" rx="9" ry="18" fill="#3a2418"/>
  <ellipsoid cx="292" cy="378" cz="15" rx="7" ry="15" rz="7"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="322" cy="370" rx="9" ry="20" fill="#3a2418"/>
  <ellipsoid cx="322" cy="370" cz="15" rx="7" ry="17" rz="7"
             fill="url(#t-arm)" filter="url(#matte)"/>

  <!-- ============== SMALL RIGHT CLAW + BAUBLE ============== -->

  <!-- Smaller right claw with a pincer tip, on the opposite side
       from the giant left claw. -->
  <ellipse cx="365" cy="285" rx="32" ry="30" fill="#3a2418"/>
  <ellipsoid cx="365" cy="285" cz="20" rx="29" ry="27" rz="27"
             fill="url(#t-arm)" filter="url(#matte)"/>
  <ellipse cx="392" cy="310" rx="15" ry="12" fill="#3a2418"/>
  <ellipsoid cx="392" cy="310" cz="25" rx="12" ry="9" rz="9"
             fill="url(#t-arm)" filter="url(#matte)"/>

  <!-- White bauble cradled inside the small right claw. -->
  <ellipse cx="395" cy="280" rx="16" ry="16" fill="#3a2418"/>
  <ellipsoid cx="395" cy="280" cz="45" rx="13" ry="13" rz="12"
             fill="url(#t-bauble)" filter="url(#gloss)"/>

  <!-- ============== EYES (interlocking, looking up-left) ============== -->

  <!-- Two tall vertical-oval eyes that touch in the middle. The
       sclera ellipsoids use `matte` (not `gloss`) so the eyeballs
       stop looking like ceramic — the catchlights and the iris
       carry the wet/shiny read instead. -->
  <ellipse cx="200" cy="225" rx="52" ry="68" fill="#3a2418"/>
  <ellipsoid cx="200" cy="225" cz="70" rx="48" ry="64" rz="48"
             fill="url(#t-eye)" filter="url(#matte)"/>
  <ellipse cx="288" cy="225" rx="52" ry="68" fill="#3a2418"/>
  <ellipsoid cx="288" cy="225" cz="70" rx="48" ry="64" rz="48"
             fill="url(#t-eye)" filter="url(#matte)"/>

  <!-- Irises shifted up-and-left from each eye centre so the pupils
       gaze up-left like the reference. They stay glossy so the
       brown reads as wet. -->
  <cylinder cx="190" cy="208" cz="115" rx="34" ry="42" depth="12"
            fill="url(#t-iris)" filter="url(#gloss)"/>
  <cylinder cx="278" cy="208" cz="115" rx="34" ry="42" depth="12"
            fill="url(#t-iris)" filter="url(#gloss)"/>

  <!-- Catchlights: a big bright primary up-and-left of each iris,
       a small secondary glint lower-right. -->
  <ellipsoid cx="170" cy="186" cz="140" rx="14" ry="18" rz="8"
             fill="#ffffff"/>
  <ellipsoid cx="258" cy="186" cz="140" rx="14" ry="18" rz="8"
             fill="#ffffff"/>
  <ellipsoid cx="202" cy="230" cz="140" rx="5" ry="5" rz="3"
             fill="#ffffff"/>
  <ellipsoid cx="290" cy="230" cz="140" rx="5" ry="5" rz="3"
             fill="#ffffff"/>

  <!-- ============== SMIRK MOUTH ============== -->

  <!-- A single curved stroked path on the body — the reference's
       subtle smirk. Replaces the open cylinder cavity + tooth cube
       that was making the previous fixture look gormless. -->
  <path d="M 215 310 Q 245 326 280 312"
        stroke="#3a2418" stroke-width="5" stroke-linecap="round"
        fill="none"/>
</svg>"##;
