//! Adopting imported nodes from the viewer (docs/design/iac-import.md,
//! "Adopt"): the background job's states, and the dialog that shows its
//! report before anything is written.
//!
//! The job is a dry run: `rhizo_iac::adopt` checks the shapes and runs
//! L1-adopt (OpenTofu, seconds to minutes) on a worker thread and returns
//! the rewritten diagram without writing it. The person reads the report
//! and presses Adopt; only then is the file written, and only if it is
//! still the text the job started from.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use rhizo_core::Color;
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, Rect, RoundedRect, Stroke, Vec2};
use vello::peniko::Fill;

use crate::chrome::{self, ACCENT, Text, ink, with_alpha};
use crate::draw::FontCache;
use crate::peniko_color;

/// What one job adopts: nodes by id, or every one that switches without a change.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    Nodes(Vec<String>),
    All,
}

impl Target {
    /// "`receipts`", "2 nodes", "every node that switches without changes".
    pub fn describe(&self) -> String {
        match self {
            Target::Nodes(ids) if ids.len() == 1 => format!("`{}`", ids[0]),
            Target::Nodes(ids) => chrome::plural(ids.len(), "node"),
            Target::All => "every node that switches without changes".into(),
        }
    }
}

/// A finished dry run, waiting for the person.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Review {
    pub target: Target,
    pub path: PathBuf,
    /// The file's text the job started from.
    pub source: String,
    /// The diagram with the nodes adopted.
    pub kdl: String,
    pub report: String,
    pub nodes: Vec<String>,
    /// How far the report is scrolled, in lines.
    pub scroll: usize,
    /// Not a report but a refusal: the providers to allow for one more run
    /// ("Allow and adopt"). Empty after a dry run that passed.
    pub allow: Vec<String>,
}

/// The one adopt job of the window.
#[derive(Debug, Default)]
pub enum Job {
    #[default]
    Idle,
    Running {
        target: Target,
        path: PathBuf,
        source: String,
        /// What OpenTofu does now ("planning the original code").
        step: String,
        cancel: Arc<AtomicBool>,
    },
    Review(Box<Review>),
}

/// What a finished job leads to.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Show the report and wait for Adopt.
    Review,
    /// `--all` found only moves: write at once, no question to ask.
    Write {
        kdl: String,
        nodes: Vec<String>,
        path: PathBuf,
        source: String,
    },
    Cancelled,
    /// It refused or failed; nothing was written.
    Failed(String),
}

impl Job {
    /// Starts a job on `path` holding `source`; refuses while one runs or waits.
    pub fn start(&mut self, target: Target, path: PathBuf, source: String) -> Result<Arc<AtomicBool>, String> {
        match self {
            Job::Idle => {}
            Job::Running { target: t, .. } => {
                return Err(format!("Adopting {} already: one adopt at a time", t.describe()));
            }
            Job::Review(_) => return Err("An adopt report is waiting: Adopt or Cancel it first".into()),
        }
        let cancel = Arc::new(AtomicBool::new(false));
        *self = Job::Running { target, path, source, step: "starting".into(), cancel: cancel.clone() };
        Ok(cancel)
    }

    pub fn step(&mut self, what: String) {
        if let Job::Running { step, cancel, .. } = self
            && !cancel.load(Ordering::Relaxed)
        {
            *step = what;
        }
    }

    /// The worker's answer: the rewritten diagram, its report, the nodes
    /// adopted and whether L1-adopt found only moves; or why not.
    pub fn finish(&mut self, result: Result<(String, String, Vec<String>, bool), Finished>) -> Outcome {
        let Job::Running { target, path, source, .. } = std::mem::take(self) else {
            return Outcome::Cancelled; // cancelled and forgotten already
        };
        match result {
            Err(Finished::Cancelled) => Outcome::Cancelled,
            Err(Finished::Failed(why)) => Outcome::Failed(why),
            Err(Finished::Refused { why, allow }) => {
                let (kdl, nodes) = (String::new(), Vec::new());
                *self =
                    Job::Review(Box::new(Review { target, path, source, kdl, report: why, nodes, scroll: 0, allow }));
                Outcome::Review
            }
            Ok((kdl, _, nodes, true)) if target == Target::All => Outcome::Write { kdl, nodes, path, source },
            Ok((kdl, report, nodes, _)) => {
                *self = Job::Review(Box::new(Review {
                    target,
                    path,
                    source,
                    kdl,
                    report,
                    nodes,
                    scroll: 0,
                    allow: vec![],
                }));
                Outcome::Review
            }
        }
    }

    /// Stops a running job (its `tofu` is killed; it answers `Cancelled`) or
    /// drops a waiting report. Nothing is written either way.
    pub fn cancel(&mut self) {
        match self {
            Job::Running { cancel, step, .. } => {
                cancel.store(true, Ordering::Relaxed);
                *step = "cancelling".into();
            }
            Job::Review(_) => *self = Job::Idle,
            Job::Idle => {}
        }
    }

    /// Adopt pressed: the diagram to write, if the file on disk (`disk`)
    /// is still what the job read.
    pub fn confirm(&mut self, disk: Option<&str>) -> Result<Box<Review>, String> {
        let Job::Review(r) = std::mem::take(self) else { return Err("Nothing to adopt".into()) };
        if disk != Some(r.source.as_str()) {
            return Err("The file changed while adopting: nothing was written; run Adopt again".into());
        }
        Ok(r)
    }

    /// The status bar's words while a job runs.
    pub fn status(&self) -> Option<String> {
        match self {
            Job::Running { target, step, .. } => Some(format!("Adopting {}… {step}", target.describe())),
            _ => None,
        }
    }

    pub fn review(&self) -> Option<&Review> {
        match self {
            Job::Review(r) => Some(r),
            _ => None,
        }
    }

    pub fn review_mut(&mut self) -> Option<&mut Review> {
        match self {
            Job::Review(r) => Some(r),
            _ => None,
        }
    }
}

/// Why a job ended without a report.
#[derive(Debug, PartialEq, Eq)]
pub enum Finished {
    Cancelled,
    Failed(String),
    /// The code requires providers outside rhizo's allow-list: `allow` is
    /// every provider a run would need allowed, `why` the refusal.
    Refused {
        why: String,
        allow: Vec<String>,
    },
}

/// A button of the report dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Button {
    Adopt,
    Cancel,
}

/// `text` broken into lines of at most `width` characters, at spaces where
/// it can; continuation lines keep the line's indent plus two.
pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let width = width.max(16);
    let mut out = Vec::new();
    for line in text.lines() {
        let indent = line.len() - line.trim_start().len();
        let mut rest: Vec<char> = line.chars().collect();
        let mut first = true;
        loop {
            let pad = if first { 0 } else { indent + 2 };
            let room = width - pad.min(width / 2);
            if rest.len() <= room {
                out.push(" ".repeat(pad) + &rest.iter().collect::<String>());
                break;
            }
            let cut = rest[..room].iter().rposition(|c| *c == ' ').filter(|&i| i > room / 2).unwrap_or(room);
            out.push(" ".repeat(pad) + &rest[..cut].iter().collect::<String>());
            rest = rest[cut..].iter().skip_while(|c| **c == ' ').copied().collect();
            first = false;
        }
    }
    out
}

/// The dialog's panel in a `size` window.
fn panel(size: Vec2, dpr: f64) -> Rect {
    let w = (760.0 * dpr).min(size.x - 32.0 * dpr);
    let h = (560.0 * dpr).min(size.y - 96.0 * dpr);
    let (x0, y0) = ((size.x - w) / 2.0, (48.0 * dpr).max((size.y - h) / 2.0 - 20.0 * dpr));
    Rect::new(x0, y0, x0 + w, y0 + h)
}

/// The report lines that fit, and how many lines there are in all.
pub fn visible(r: &Review, mono: &Fonts, size: Vec2, dpr: f64) -> (Vec<String>, usize, usize) {
    let p = panel(size, dpr);
    let pt = (12.0 * dpr) as f32;
    let advance = mono.shape("0000000000", pt, 400).map_or(7.2 * dpr, |s| s.width as f64 / 10.0);
    let lines = wrap(&r.report, ((p.width() - 48.0 * dpr) / advance) as usize);
    let rows = ((p.height() - 150.0 * dpr) / (17.0 * dpr)).max(1.0) as usize;
    let first = r.scroll.min(lines.len().saturating_sub(rows));
    let total = lines.len();
    (lines.into_iter().skip(first).take(rows).collect(), first, total)
}

/// Draws the report of `r` over a `size` window, dimming it; returns where
/// the buttons are. `hover` lights a button.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    ui: &Fonts,
    mono: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    r: &Review,
    hover: Option<Button>,
) -> Vec<(Rect, Button)> {
    let ink = ink(bg);
    let px = |v: f64| v * dpr;
    s.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        peniko_color(with_alpha(Color::rgb(0, 0, 0), 0.28)),
        None,
        &Rect::new(0.0, 0.0, size.x, size.y),
    );
    let p = panel(size, dpr);
    chrome::shadow(s, p, px(12.0), dpr);
    let rr = RoundedRect::from_rect(p, px(12.0));
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg), None, &rr);
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink.mix(bg, 0.8)), None, &rr);
    let (x0, x1) = (p.x0 + px(24.0), p.x1 - px(24.0));
    let faint = ink.mix(bg, 0.45);
    let mut t = Text { s, fonts: ui, cache, size: px(16.0) as f32, baseline: (p.y0 + px(36.0)) as f32 };
    let title = match &r.target {
        _ if !r.allow.is_empty() => format!("Allow {} for this adopt?", r.allow.join(", ")),
        Target::All => format!("Adopt {}?", chrome::plural(r.nodes.len(), "node")),
        Target::Nodes(_) => format!("Adopt {}?", Target::Nodes(r.nodes.clone()).describe()),
    };
    let title = t.clip(&title, 600, x1 - x0);
    t.draw(&title, x0, 600, ink);
    t.size = px(12.5) as f32;
    t.baseline = (p.y0 + px(58.0)) as f32;
    let sub = if r.allow.is_empty() {
        "L1-adopt passed offline (mocked providers). Nothing is written until you press Adopt."
    } else {
        "Nothing was installed. Allow and adopt runs these providers on this machine, for this run only."
    };
    let sub = t.clip(sub, 400, x1 - x0);
    t.draw(&sub, x0, 400, faint);

    // The report, in the text pane's font.
    let body = Rect::new(p.x0 + px(16.0), p.y0 + px(76.0), p.x1 - px(16.0), p.y1 - px(64.0));
    let well = bg.mix(ink, 0.04);
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(well), None, &RoundedRect::from_rect(body, px(8.0)));
    let (lines, first, total) = visible(r, mono, size, dpr);
    let mut m = Text { s: t.s, fonts: mono, cache: t.cache, size: px(12.0) as f32, baseline: 0.0 };
    m.s.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &body);
    for (i, line) in lines.iter().enumerate() {
        m.baseline = (body.y0 + px(20.0) + px(17.0) * i as f64) as f32;
        let tone = if line.trim_start().starts_with("adopting ") || line.contains("would destroy") {
            ink.mix(chrome::WARNING, 0.35)
        } else if line.starts_with("Live") {
            ACCENT.mix(ink, 0.2)
        } else {
            ink.mix(bg, 0.12)
        };
        m.draw(line, x0, 400, tone);
    }
    m.s.pop_layer();
    let mut t = Text { s: m.s, fonts: ui, cache: m.cache, size: px(12.0) as f32, baseline: 0.0 };
    if total > lines.len() {
        t.baseline = (p.y1 - px(30.0)) as f32;
        let more = format!("lines {}–{} of {total} · scroll for more", first + 1, first + lines.len());
        t.draw(&more, x0, 400, faint);
    }

    // Cancel, then Adopt on the right.
    let mut hits = Vec::new();
    let mut right = x1;
    let adopt = if r.allow.is_empty() { "Adopt" } else { "Allow and adopt" };
    for (b, label, keys) in [(Button::Adopt, adopt, "⌘Enter"), (Button::Cancel, "Cancel", "Esc")] {
        t.size = px(13.0) as f32;
        let keys = chrome::keys(t.fonts, keys);
        t.size = px(11.0) as f32;
        let kw = t.width(&keys, 400);
        t.size = px(13.0) as f32;
        let w = t.width(label, 600) + kw + px(36.0);
        let r = Rect::new(right - w, p.y1 - px(48.0), right, p.y1 - px(16.0));
        let lit = hover == Some(b);
        let (fill, fg) = match b {
            Button::Adopt => (if lit { ACCENT.mix(ink, 0.15) } else { ACCENT }, Color::hex(0xffffff)),
            Button::Cancel => (with_alpha(ink, if lit { 0.12 } else { 0.07 }), ink),
        };
        t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &RoundedRect::from_rect(r, px(8.0)));
        t.baseline = (r.center().y + px(13.0) * 0.35) as f32;
        let lw = t.draw(label, r.x0 + px(14.0), 600, fg);
        t.size = px(11.0) as f32;
        t.draw(&keys, r.x0 + px(22.0) + lw, 400, fg.mix(fill, 0.35));
        hits.push((r, b));
        right = r.x0 - px(10.0);
    }
    hits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn review_of(job: &mut Job, quiet: bool) -> Outcome {
        job.finish(Ok(("new".into(), "rhizo adopt\n".into(), vec!["b".into()], quiet)))
    }

    #[test]
    fn one_job_at_a_time_and_a_review_before_writing() {
        let mut job = Job::default();
        let cancel = job.start(Target::Nodes(vec!["b".into()]), "/x.rhizo".into(), "old".into()).unwrap();
        assert!(!cancel.load(Ordering::Relaxed));
        assert!(job.start(Target::All, "/x.rhizo".into(), "old".into()).unwrap_err().contains("one adopt at a time"));
        job.step("planning the original code".into());
        assert_eq!(job.status().as_deref(), Some("Adopting `b`… planning the original code"));
        // A dry run by name always waits for Adopt, even with only moves.
        assert_eq!(review_of(&mut job, true), Outcome::Review);
        assert!(job.status().is_none() && job.review().is_some());
        assert!(job.start(Target::All, "/x.rhizo".into(), "old".into()).unwrap_err().contains("waiting"));
        // The file changed meanwhile: nothing to write, the report is gone.
        assert!(job.confirm(Some("edited")).unwrap_err().contains("changed"));
        assert!(matches!(job, Job::Idle));
        job.start(Target::Nodes(vec!["b".into()]), "/x.rhizo".into(), "old".into()).unwrap();
        review_of(&mut job, false);
        let r = job.confirm(Some("old")).unwrap();
        assert_eq!((r.kdl.as_str(), r.nodes.as_slice()), ("new", &["b".to_string()][..]));
        assert!(matches!(job, Job::Idle));
    }

    #[test]
    fn all_with_only_moves_writes_without_asking() {
        let mut job = Job::default();
        job.start(Target::All, "/x.rhizo".into(), "old".into()).unwrap();
        assert!(matches!(review_of(&mut job, true), Outcome::Write { ref kdl, .. } if kdl == "new"));
        assert!(matches!(job, Job::Idle));
        job.start(Target::All, "/x.rhizo".into(), "old".into()).unwrap();
        assert_eq!(review_of(&mut job, false), Outcome::Review, "changes or creates: ask");
    }

    #[test]
    fn cancelling_kills_the_run_and_writes_nothing() {
        let mut job = Job::default();
        let flag = job.start(Target::All, "/x.rhizo".into(), "old".into()).unwrap();
        job.cancel();
        assert!(flag.load(Ordering::Relaxed), "the worker's tofu is told to stop");
        job.step("planning the adopted code".into());
        assert_eq!(job.status().as_deref(), Some("Adopting every node that switches without changes… cancelling"));
        assert_eq!(job.finish(Err(Finished::Cancelled)), Outcome::Cancelled);
        assert!(matches!(job, Job::Idle));
        job.start(Target::All, "/x.rhizo".into(), "old".into()).unwrap();
        assert_eq!(job.finish(Err(Finished::Failed("no".into()))), Outcome::Failed("no".into()));
        job.start(Target::Nodes(vec!["b".into()]), "/x.rhizo".into(), "old".into()).unwrap();
        review_of(&mut job, false);
        job.cancel();
        assert!(matches!(job, Job::Idle), "a waiting report is dropped");
        assert_eq!(job.finish(Err(Finished::Cancelled)), Outcome::Cancelled, "a late answer changes nothing");
    }

    #[test]
    fn a_provider_refusal_waits_for_allow_and_writes_nothing() {
        let mut job = Job::default();
        job.start(Target::All, "/x.rhizo".into(), "old".into()).unwrap();
        let allow = vec!["integrations/github".to_string()];
        let refused = Finished::Refused { why: "refusing to run OpenTofu".into(), allow: allow.clone() };
        assert_eq!(job.finish(Err(refused)), Outcome::Review);
        let r = job.review().unwrap();
        assert_eq!((r.allow.as_slice(), r.kdl.as_str(), r.target.clone()), (allow.as_slice(), "", Target::All));
        job.cancel();
        assert!(matches!(job, Job::Idle), "Cancel drops it; nothing is allowed later");
    }

    #[test]
    fn report_lines_wrap_at_spaces_with_their_indent() {
        let w = wrap("  adopting changes module.b.x: tags: {} → {\"a\":\"1\"}\nshort", 24);
        assert!(w.iter().all(|l| l.chars().count() <= 24), "{w:?}");
        assert_eq!(w[0], "  adopting changes");
        assert!(w[1].starts_with("    module."), "{w:?}");
        assert_eq!(w.last().map(String::as_str), Some("short"));
        assert_eq!(wrap("", 40), Vec::<String>::new());
    }
}
