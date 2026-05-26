//! Dashed-stroke slicing.
//!
//! Splits a flattened path into the visible portions of a `stroke-dasharray`
//! pattern. Lyon tessellates the resulting derived path with the same stroke
//! cap/join/miter options as the solid case.

use lyon_tessellation::path::math::Point;
use lyon_tessellation::path::{Path, PathEvent};

use super::{EPSILON, FLATTENING_TOLERANCE};
use lyon_tessellation::path::iterator::PathIterator;

/// A repeated dash/gap pattern in user units, with a starting offset.
#[derive(Debug, Clone)]
pub(super) struct DashPattern {
    values: Vec<f32>,
    offset: f32,
    period: f32,
}

impl DashPattern {
    /// Wrap a dash-array list into a pattern. Returns `None` for a zero-length
    /// period (which would loop forever).
    pub(super) fn new(values: Vec<f32>, offset: f32) -> Option<Self> {
        let period: f32 = values.iter().sum();
        (period > EPSILON).then_some(Self {
            values,
            offset,
            period,
        })
    }

    #[cfg(test)]
    pub(super) fn values(&self) -> &[f32] {
        &self.values
    }

    #[cfg(test)]
    pub(super) fn offset(&self) -> f32 {
        self.offset
    }

    fn cursor(&self) -> DashCursor<'_> {
        let mut distance = self.offset.rem_euclid(self.period);
        let mut index = 0;
        while distance > self.values[index] && self.values[index] > EPSILON {
            distance -= self.values[index];
            index = (index + 1) % self.values.len();
        }
        DashCursor {
            pattern: self,
            index,
            remaining: (self.values[index] - distance).max(0.0),
        }
    }
}

struct DashCursor<'a> {
    pattern: &'a DashPattern,
    index: usize,
    remaining: f32,
}

impl DashCursor<'_> {
    fn is_painting(&self) -> bool {
        self.index.is_multiple_of(2)
    }

    fn advance(&mut self, mut distance: f32) {
        while distance > EPSILON {
            if self.remaining <= EPSILON {
                self.next();
                continue;
            }
            let step = distance.min(self.remaining);
            self.remaining -= step;
            distance -= step;
            if self.remaining <= EPSILON {
                self.next();
            }
        }
    }

    fn next(&mut self) {
        self.index = (self.index + 1) % self.pattern.values.len();
        self.remaining = self.pattern.values[self.index];
    }
}

/// One subpath, flattened into a polyline.
#[derive(Debug, Clone)]
pub(super) struct FlatSubpath {
    points: Vec<Point>,
}

impl FlatSubpath {
    /// Cumulative length of the polyline.
    pub(super) fn length(&self) -> f32 {
        self.points
            .windows(2)
            .map(|pair| (pair[1] - pair[0]).length())
            .sum()
    }
}

/// Build a derived path made of the *painted* dash portions of `path`,
/// preserving subpath structure where possible. Curves are flattened first.
pub(super) fn dashed_path(path: &Path, pattern: &DashPattern) -> Path {
    let mut builder = Path::builder().with_svg();

    for subpath in flattened_subpaths(path) {
        let mut cursor = pattern.cursor();
        let mut open_dash = false;

        for pair in subpath.points.windows(2) {
            let mut current = pair[0];
            let to = pair[1];
            let vector = to - current;
            let length = vector.length();
            if length <= EPSILON {
                continue;
            }
            let direction = vector / length;
            let mut remaining = length;

            while remaining > EPSILON {
                if cursor.remaining <= EPSILON {
                    cursor.next();
                }
                let step = remaining.min(cursor.remaining);
                let next = current + direction * step;

                if cursor.is_painting() {
                    if !open_dash {
                        builder.move_to(current);
                        open_dash = true;
                    }
                    builder.line_to(next);
                } else if open_dash {
                    open_dash = false;
                }

                current = next;
                remaining -= step;
                cursor.advance(step);
            }
        }
    }

    builder.build()
}

/// Flatten the path and return one [`FlatSubpath`] per `M…Z?` run. Quadratic
/// and cubic curves are flattened with [`FLATTENING_TOLERANCE`]; explicit
/// close events extend the run back to the subpath start.
pub(super) fn flattened_subpaths(path: &Path) -> Vec<FlatSubpath> {
    let mut subpaths = Vec::new();
    let mut current: Vec<Point> = Vec::new();

    for event in path.iter().flattened(FLATTENING_TOLERANCE) {
        match event {
            PathEvent::Begin { at } => {
                flush_flat_subpath(&mut current, &mut subpaths);
                current.push(at);
            }
            PathEvent::Line { to, .. } => {
                if current
                    .last()
                    .is_none_or(|last| (*last - to).length() > EPSILON)
                {
                    current.push(to);
                }
            }
            PathEvent::End { first, close, .. } => {
                if close
                    && current
                        .last()
                        .is_some_and(|last| (*last - first).length() > EPSILON)
                {
                    current.push(first);
                }
                flush_flat_subpath(&mut current, &mut subpaths);
            }
            PathEvent::Quadratic { .. } | PathEvent::Cubic { .. } => unreachable!(),
        }
    }
    flush_flat_subpath(&mut current, &mut subpaths);

    subpaths
}

fn flush_flat_subpath(current: &mut Vec<Point>, subpaths: &mut Vec<FlatSubpath>) {
    if current.len() >= 2 {
        subpaths.push(FlatSubpath {
            points: std::mem::take(current),
        });
    } else {
        current.clear();
    }
}
