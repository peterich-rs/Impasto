//! Span bounds in paragraph space. Glyph edges come from the caller.

use crate::LineMetric;

/// Paragraph-local rectangle. `x` and `y` are the top-left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct SpanBox {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// Axis-aligned union of the line boxes `start..end` occupies.
///
/// `cluster` returns the left and right edges of the glyph at a UTF-16 index.
/// A missing cluster uses that line's full width. An empty union is `None`.
pub(crate) fn span_bounds(
    lines: &[LineMetric],
    start: u32,
    end: u32,
    mut cluster: impl FnMut(u32) -> Option<(f32, f32)>,
) -> Option<SpanBox> {
    let mut union: Option<(f32, f32, f32, f32)> = None;
    for line in lines {
        if line.end <= start || line.start >= end {
            continue;
        }
        let left = start.max(line.start);
        let right = end.min(line.end);
        if left >= right {
            continue;
        }
        let (x0, x1) = match (cluster(left), cluster(right - 1)) {
            (Some((start_x, _)), Some((_, end_x))) => (start_x.min(end_x), start_x.max(end_x)),
            _ => {
                let x1 = line.x + line.w;
                (line.x.min(x1), line.x.max(x1))
            }
        };
        let y0 = line.y;
        let y1 = line.y + line.h;
        union = Some(match union {
            None => (x0, y0, x1, y1),
            Some((ax0, ay0, ax1, ay1)) => (ax0.min(x0), ay0.min(y0), ax1.max(x1), ay1.max(y1)),
        });
    }
    let (x0, y0, x1, y1) = union?;
    let width = x1 - x0;
    let height = y1 - y0;
    (width > 0.0 && height > 0.0).then_some(SpanBox {
        x: x0,
        y: y0,
        w: width,
        h: height,
    })
}

#[cfg(test)]
mod tests {
    use super::{SpanBox, span_bounds};
    use crate::LineMetric;

    fn line(start: u32, end: u32, x: f32, y: f32, w: f32, h: f32) -> LineMetric {
        LineMetric {
            start,
            end,
            end_with_break: end,
            x,
            y,
            w,
            h,
            baseline: h * 0.8,
            hard_break: false,
        }
    }

    #[test]
    fn a_wrapped_span_unions_each_line_slice() {
        let lines = [
            line(0, 4, 0.0, 0.0, 40.0, 10.0),
            line(4, 8, 0.0, 10.0, 40.0, 10.0),
        ];
        let bounds = span_bounds(&lines, 2, 6, |unit| {
            let left = u16::try_from(unit).unwrap_or(0);
            Some((f32::from(left) * 10.0, f32::from(left + 1) * 10.0))
        });
        assert_eq!(
            bounds,
            Some(SpanBox {
                x: 20.0,
                y: 0.0,
                w: 40.0,
                h: 20.0,
            })
        );
    }

    #[test]
    fn a_zero_width_span_has_no_box() {
        let lines = [line(0, 2, 0.0, 0.0, 0.0, 10.0)];
        assert_eq!(span_bounds(&lines, 0, 2, |_| Some((0.0, 0.0))), None);
    }
}
