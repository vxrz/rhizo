//! Sequence diagrams: participants in a row, a lifeline under each, and
//! messages stacked top to bottom in the order given.
//!
//! Columns start at their boxes' widths plus a gap and only ever widen:
//! each message's label must fit between the lifelines it joins, so
//! constraints are applied shortest span first and any shortfall is spread
//! evenly over the gaps the message crosses. Rows are as tall as their
//! labels. Everything is a pure function of the input order.

use rhizo_core::{Point, Rect, Size};

use crate::Path;

pub struct Input<'a> {
    /// Outer size of each participant's box.
    pub size: &'a [Size],
    /// `(from, to)` participant of each message, top to bottom.
    pub messages: &'a [(usize, usize)],
    /// Label size of each message, if it has one.
    pub labels: &'a [Option<Size>],
}

pub struct Output {
    /// Participant boxes, top-aligned and of equal height.
    pub boxes: Vec<Rect>,
    /// Start and end of each participant's lifeline.
    pub lifelines: Vec<(Point, Point)>,
    pub paths: Vec<Path>,
    pub label_at: Vec<Option<Point>>,
}

/// Least clear space between neighboring boxes.
const BOX_GAP: f32 = 48.0;
/// Clear space on each side of a label between the lifelines it spans.
const LABEL_PAD: f32 = 14.0;
/// Box bottom to the first message.
const TOP_GAP: f32 = 22.0;
/// One message's arrow to the next message's label.
const ROW_GAP: f32 = 18.0;
/// A label's bottom to its arrow.
const LABEL_LIFT: f32 = 2.0;
/// Extent of a self call's loop.
const SELF_W: f32 = 34.0;
const SELF_H: f32 = 24.0;
/// A self call's label starts this far right of its lifeline.
const SELF_LABEL_DX: f32 = 8.0;
/// Last message to the end of the lifelines.
const END_GAP: f32 = 24.0;

pub fn layout(input: &Input) -> Output {
    let n = input.size.len();
    let height = input.size.iter().map(|s| s.height).fold(0.0, f32::max);

    // gaps[k]: distance between the lifelines of participants k and k + 1.
    let mut gaps: Vec<f32> = (1..n).map(|k| (input.size[k - 1].width + input.size[k].width) / 2.0 + BOX_GAP).collect();
    let mut needs: Vec<(usize, usize, f32)> = Vec::new();
    for (&(a, b), label) in input.messages.iter().zip(input.labels) {
        let w = label.map_or(0.0, |s| s.width);
        if a == b {
            // The loop and its label must clear the next lifeline.
            if a + 1 < n {
                needs.push((a, a + 1, SELF_W.max(SELF_LABEL_DX + w) + LABEL_PAD));
            }
        } else {
            needs.push((a.min(b), a.max(b), w + 2.0 * LABEL_PAD));
        }
    }
    needs.sort_by_key(|&(lo, hi, _)| (hi - lo, lo));
    for (lo, hi, need) in needs {
        let have: f32 = gaps[lo..hi].iter().sum();
        if have < need {
            let extra = (need - have) / (hi - lo) as f32;
            for g in &mut gaps[lo..hi] {
                *g += extra;
            }
        }
    }

    let mut x = Vec::with_capacity(n);
    let mut at = input.size.first().map_or(0.0, |s| s.width / 2.0);
    for k in 0..n {
        x.push(at);
        at += gaps.get(k).copied().unwrap_or(0.0);
    }
    let boxes: Vec<Rect> =
        (0..n).map(|k| Rect::new(x[k] - input.size[k].width / 2.0, 0.0, input.size[k].width, height)).collect();

    let mut y = height + TOP_GAP;
    let mut paths = Vec::with_capacity(input.messages.len());
    let mut label_at = Vec::with_capacity(input.messages.len());
    for (&(a, b), label) in input.messages.iter().zip(input.labels) {
        let arrow = y + label.map_or(0.0, |s| s.height + LABEL_LIFT);
        label_at.push(label.map(|s| {
            let cx = if a == b { x[a] + SELF_LABEL_DX + s.width / 2.0 } else { (x[a] + x[b]) / 2.0 };
            Point::new(cx, arrow - LABEL_LIFT - s.height / 2.0)
        }));
        if a == b {
            let (l, r) = (x[a], x[a] + SELF_W);
            let pts = [
                Point::new(l, arrow),
                Point::new(r, arrow),
                Point::new(r, arrow + SELF_H),
                Point::new(l, arrow + SELF_H),
            ];
            paths.push(crate::rounded_polyline(&pts, 6.0));
            y = arrow + SELF_H + ROW_GAP;
        } else {
            paths.push(Path::line(Point::new(x[a], arrow), Point::new(x[b], arrow)));
            y = arrow + ROW_GAP;
        }
    }
    let end = y - ROW_GAP + END_GAP;
    let lifelines = (0..n).map(|k| (Point::new(x[k], height), Point::new(x[k], end))).collect();
    Output { boxes, lifelines, paths, label_at }
}
