//! Marker placement on the original (non-dashed) shape path.
//!
//! Computes `MarkerPlacement`s from a path's authored vertices. Curves
//! contribute markers at their endpoints with orientation derived from
//! endpoint tangents. The dash slicer in [`super::dash`] does not affect
//! marker geometry — SVG specifies that markers attach to the original
//! shape path, not to individual dashes.

use lyon_tessellation::path::math::{Point, Vector};
use lyon_tessellation::path::{Path, PathEvent};

use super::EPSILON;

/// A marker placement on a stroked path.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct MarkerPlacement {
    /// Which marker property this placement consumes.
    pub(crate) kind: MarkerKind,
    /// Marker origin x coordinate in user units.
    pub(crate) x: f32,
    /// Marker origin y coordinate in user units.
    pub(crate) y: f32,
    /// Auto-orientation angle in radians.
    pub(crate) angle: f32,
}

/// Marker property slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MarkerKind {
    Start,
    Mid,
    End,
}

/// Compute marker placements from a path's authored vertices.
///
/// Curves contribute markers at their endpoints, with orientation derived
/// from endpoint tangents. This intentionally does not use dash slicing:
/// markers attach to the original shape path, not to individual dashes.
pub(crate) fn marker_placements(path: &Path) -> Vec<MarkerPlacement> {
    let mut placements = Vec::new();
    for subpath in marker_subpaths(path) {
        if subpath.vertices.len() < 2 {
            continue;
        }

        let first = subpath.vertices[0];
        placements.push(MarkerPlacement {
            kind: MarkerKind::Start,
            x: first.point.x,
            y: first.point.y,
            angle: angle(first.outgoing.or(first.incoming)),
        });

        let last_index = subpath.vertices.len() - 1;
        for vertex in &subpath.vertices[1..last_index] {
            placements.push(MarkerPlacement {
                kind: MarkerKind::Mid,
                x: vertex.point.x,
                y: vertex.point.y,
                angle: marker_angle(vertex.incoming, vertex.outgoing),
            });
        }

        let last = subpath.vertices[last_index];
        placements.push(MarkerPlacement {
            kind: MarkerKind::End,
            x: last.point.x,
            y: last.point.y,
            angle: angle(last.incoming.or(last.outgoing)),
        });
    }
    placements
}

#[derive(Debug, Clone)]
struct MarkerSubpath {
    vertices: Vec<MarkerVertex>,
}

#[derive(Debug, Clone, Copy)]
struct MarkerVertex {
    point: Point,
    incoming: Option<Vector>,
    outgoing: Option<Vector>,
}

fn marker_subpaths(path: &Path) -> Vec<MarkerSubpath> {
    let mut subpaths = Vec::new();
    let mut current: Vec<MarkerVertex> = Vec::new();

    for event in path.iter() {
        match event {
            PathEvent::Begin { at } => {
                flush_marker_subpath(&mut current, &mut subpaths);
                current.push(MarkerVertex {
                    point: at,
                    incoming: None,
                    outgoing: None,
                });
            }
            PathEvent::Line { from, to } => {
                push_marker_segment(&mut current, from, to, to - from, to - from);
            }
            PathEvent::Quadratic { from, ctrl, to } => {
                push_marker_segment(
                    &mut current,
                    from,
                    to,
                    non_zero_tangent(ctrl - from, to - from),
                    non_zero_tangent(to - ctrl, to - from),
                );
            }
            PathEvent::Cubic {
                from,
                ctrl1,
                ctrl2,
                to,
            } => {
                push_marker_segment(
                    &mut current,
                    from,
                    to,
                    non_zero_tangent(ctrl1 - from, to - from),
                    non_zero_tangent(to - ctrl2, to - from),
                );
            }
            PathEvent::End { first, last, close } => {
                if close && (first - last).length() > EPSILON {
                    let dir = first - last;
                    let incoming = normalized(dir);
                    let outgoing = current.first().and_then(|vertex| vertex.outgoing);
                    if let Some(last_vertex) = current.last_mut() {
                        last_vertex.outgoing = incoming;
                    }
                    current.push(MarkerVertex {
                        point: first,
                        incoming,
                        outgoing,
                    });
                }
                flush_marker_subpath(&mut current, &mut subpaths);
            }
        }
    }
    flush_marker_subpath(&mut current, &mut subpaths);

    subpaths
}

fn push_marker_segment(
    current: &mut Vec<MarkerVertex>,
    from: Point,
    to: Point,
    outgoing: Vector,
    incoming: Vector,
) {
    if current.is_empty() {
        current.push(MarkerVertex {
            point: from,
            incoming: None,
            outgoing: None,
        });
    }

    if let Some(last) = current.last_mut() {
        last.outgoing = normalized(outgoing);
    }
    current.push(MarkerVertex {
        point: to,
        incoming: normalized(incoming),
        outgoing: None,
    });
}

fn flush_marker_subpath(current: &mut Vec<MarkerVertex>, subpaths: &mut Vec<MarkerSubpath>) {
    if current.len() >= 2 {
        subpaths.push(MarkerSubpath {
            vertices: std::mem::take(current),
        });
    } else {
        current.clear();
    }
}

fn non_zero_tangent(primary: Vector, fallback: Vector) -> Vector {
    if primary.length() > EPSILON {
        primary
    } else {
        fallback
    }
}

fn normalized(vector: Vector) -> Option<Vector> {
    (vector.length() > EPSILON).then(|| vector.normalize())
}

fn marker_angle(incoming: Option<Vector>, outgoing: Option<Vector>) -> f32 {
    match (incoming, outgoing) {
        (Some(a), Some(b)) => {
            let sum = a + b;
            if sum.length() > EPSILON {
                angle(Some(sum))
            } else {
                angle(Some(b))
            }
        }
        (Some(vector), None) | (None, Some(vector)) => angle(Some(vector)),
        (None, None) => 0.0,
    }
}

fn angle(vector: Option<Vector>) -> f32 {
    vector
        .map(|vector| vector.y.atan2(vector.x))
        .unwrap_or_default()
}
