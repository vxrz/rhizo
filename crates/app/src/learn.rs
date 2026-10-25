//! "Learn rhizo": the tutorial starter (`rhizo new learn`) and the card
//! that ticks its steps off as they are done. What was done is read from
//! the document, against the starter it began as, and from a few things
//! the window saw happen (compiling, the text pane, the palette), so a
//! tutorial file opened again later shows the steps it already holds.

use rhizo_core::{Color, Document};
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, Circle, Rect, RoundedRect, Stroke};
use vello::peniko::Fill;

use crate::chrome::{self, ACCENT, Text};
use crate::draw::FontCache;
use crate::palette::{self, Action};
use crate::peniko_color;

/// The tutorial's title: a document with it is the tutorial.
pub const TITLE: &str = "Learn rhizo";

/// How a step is known to be done.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Check {
    /// A new node under another.
    Child,
    /// A new node beside another, under the same parent or at the top of the same frame.
    Sibling,
    /// A node or frame of the starter has a new label.
    Rename,
    Edge,
    /// A new frame holding two nodes or more.
    Group,
    /// A node that had no type has one.
    Typed,
    /// A frame typed `infra.environment` with a cloud and a region (what it needs to compile).
    Environment,
    Compiled,
    Pane,
    Palette,
}

/// One step: what to do, how, and the registry command whose keys do it.
pub struct Step {
    pub title: &'static str,
    pub how: &'static str,
    pub action: Action,
    check: Check,
}

pub const STEPS: &[Step] = &[
    Step { title: "Add a child", how: "Select “My idea”, then Tab", action: Action::AddChild, check: Check::Child },
    Step { title: "Add a sibling", how: "Enter on a node", action: Action::AddSibling, check: Check::Sibling },
    Step { title: "Rename a node", how: "F2 or double-click it", action: Action::Rename, check: Check::Rename },
    Step {
        title: "Connect two nodes",
        how: "Alt+drag from Apples to Pears",
        action: Action::Connect,
        check: Check::Edge,
    },
    Step {
        title: "Group nodes into a frame",
        how: "Shift+click both fruits, or drag a box",
        action: Action::GroupFrame,
        check: Check::Group,
    },
    Step {
        title: "Give a node a type",
        how: "In the inspector, or ⌘K Change type…",
        action: Action::ToggleInspector,
        check: Check::Typed,
    },
    Step {
        title: "Make an environment",
        how: "New frame, type Environment; cloud and region in ⌘I",
        action: Action::FrameTypeMenu,
        check: Check::Environment,
    },
    Step { title: "Compile it", how: "Diagram ▸ Compile specs…", action: Action::Compile, check: Check::Compiled },
    Step { title: "Read the text", how: "The same diagram as text", action: Action::TogglePane, check: Check::Pane },
    Step {
        title: "Find any command",
        how: "Try “environment” or “export”",
        action: Action::Palette,
        check: Check::Palette,
    },
];

/// What the window saw happen during the tutorial.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Seen {
    pub compiled: bool,
    pub pane: bool,
    pub palette: bool,
}

/// The tutorial as it starts: `rhizo new learn`.
pub fn start() -> Document {
    let src = rhizo_lang::starters::starter("learn").expect("the learn starter");
    rhizo_lang::load(src, rhizo_lang::Syntax::Kdl).document
}

pub fn is_tutorial(doc: &Document) -> bool {
    doc.title.as_deref() == Some(TITLE)
}

/// Which of [`STEPS`] `now` and `seen` show done, against `start`.
pub fn progress(start: &Document, now: &Document, seen: Seen) -> Vec<bool> {
    let new: Vec<&rhizo_core::Node> = now.nodes.values().filter(|n| !start.nodes.contains_key(&n.id)).collect();
    let beside = |n: &rhizo_core::Node| {
        now.nodes.values().any(|m| m.id != n.id && m.parent == n.parent && (n.parent.is_some() || m.frame == n.frame))
    };
    STEPS
        .iter()
        .map(|s| match s.check {
            Check::Child => new.iter().any(|n| n.parent.is_some()),
            Check::Sibling => new.iter().any(|n| beside(n)),
            Check::Rename => {
                start.nodes.values().any(|n| now.node(&n.id).is_some_and(|m| m.label != n.label))
                    || start.frames.iter().any(|f| now.frame(&f.id).is_some_and(|g| g.label != f.label))
            }
            Check::Edge => now.edges.len() > start.edges.len(),
            Check::Group => now
                .frames
                .iter()
                .any(|f| start.frame(&f.id).is_none() && now.nodes.values().filter(|n| n.frame == f.id).count() >= 2),
            Check::Typed => {
                now.nodes.values().any(|n| n.ty.is_some() && start.node(&n.id).is_none_or(|m| m.ty.is_none()))
            }
            Check::Environment => now.frames.iter().any(|f| {
                f.ty.as_ref().is_some_and(|t| t.as_str() == "infra.environment")
                    && ["cloud", "region"].iter().all(|k| f.props.contains_key(*k))
            }),
            Check::Compiled => seen.compiled,
            Check::Pane => seen.pane,
            Check::Palette => seen.palette,
        })
        .collect()
}

/// "4 of 10".
pub fn count(done: &[bool]) -> String {
    format!("{} of {}", done.iter().filter(|d| **d).count(), done.len())
}

/// The card in the bottom-right corner of `canvas` (window pixels, above
/// the status bar), where the tutorial's diagram leaves room: the
/// steps with their ticks, and how to do the next one. Returns where it is.
#[allow(clippy::too_many_arguments)]
pub fn card(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    canvas: Rect,
    dpr: f64,
    bg: Color,
    done: &[bool],
) -> Rect {
    let ink = chrome::ink(bg);
    let px = |v: f64| v * dpr;
    let next = done.iter().position(|d| !d);
    let (pad, line) = (px(14.0), px(21.0));
    let w = px(296.0).min(canvas.width() - px(24.0));
    let h = pad + px(26.0) + px(10.0) + line * STEPS.len() as f64 + px(if next.is_some() { 26.0 } else { 6.0 }) + pad;
    let x0 = canvas.x1 - px(12.0) - w;
    let y0 = (canvas.y1 - px(12.0) - h).max(canvas.y0 + px(12.0));
    let panel = Rect::new(x0, y0, x0 + w, y0 + h);
    let rr = RoundedRect::from_rect(panel, px(10.0));
    chrome::shadow(s, panel, px(10.0), dpr);
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg.mix(ink, 0.025)), None, &rr);
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink.mix(bg, 0.82)), None, &rr);

    let mut t = Text { s, fonts, cache, size: px(13.5) as f32, baseline: (y0 + pad + px(13.0)) as f32 };
    t.draw(TITLE, x0 + pad, 650, ink);
    let n = count(done);
    t.size = px(12.0) as f32;
    let nw = t.width(&n, 500);
    t.draw(&n, x0 + w - pad - nw, 500, ink.mix(bg, 0.4));
    // The bar: done over all.
    let bar_y = y0 + pad + px(24.0);
    let full = Rect::new(x0 + pad, bar_y, x0 + w - pad, bar_y + px(3.0));
    let part = done.iter().filter(|d| **d).count() as f64 / done.len().max(1) as f64;
    t.s.fill(
        Fill::NonZero,
        Affine::IDENTITY,
        peniko_color(ink.mix(bg, 0.88)),
        None,
        &RoundedRect::from_rect(full, 1.5),
    );
    let filled = Rect::new(full.x0, full.y0, full.x0 + full.width() * part, full.y1);
    t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(ACCENT), None, &RoundedRect::from_rect(filled, 1.5));

    let mut y = bar_y + px(10.0);
    for (i, (step, &ok)) in STEPS.iter().zip(done).enumerate() {
        let mid = y + line / 2.0;
        let dot = Circle::new((x0 + pad + px(6.0), mid), px(6.0));
        if ok {
            t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(ACCENT), None, &dot);
            let mut tick = vello::kurbo::BezPath::new();
            tick.move_to((dot.center.x - px(3.0), mid));
            tick.line_to((dot.center.x - px(0.8), mid + px(2.2)));
            tick.line_to((dot.center.x + px(3.0), mid - px(2.2)));
            t.s.stroke(&Stroke::new(px(1.6)), Affine::IDENTITY, peniko_color(Color::rgb(255, 255, 255)), None, &tick);
        } else {
            let tone = if next == Some(i) { ACCENT } else { ink.mix(bg, 0.65) };
            t.s.stroke(
                &Stroke::new(px(1.3)),
                Affine::IDENTITY,
                peniko_color(tone),
                None,
                &Circle::new(dot.center, px(5.4)),
            );
        }
        let is_next = next == Some(i);
        t.size = px(12.5) as f32;
        t.baseline = (mid + px(12.5) * 0.35) as f32;
        let keys = palette::command(step.action).map_or("", |c| c.keys);
        let keys = chrome::keys(t.fonts, keys);
        let kw = t.width(&keys, 400);
        let room = w - 2.0 * pad - px(20.0) - kw - px(8.0);
        let title = t.clip(&format!("{}. {}", i + 1, step.title), if is_next { 600 } else { 400 }, room);
        let tone = match () {
            _ if ok => ink.mix(bg, 0.5),
            _ if is_next => ink,
            _ => ink.mix(bg, 0.25),
        };
        t.draw(&title, x0 + pad + px(20.0), if is_next { 600 } else { 400 }, tone);
        t.draw(&keys, x0 + w - pad - kw, 400, if is_next { ACCENT.mix(ink, 0.2) } else { ink.mix(bg, 0.55) });
        y += line;
    }
    if let Some(i) = next {
        t.size = px(12.0) as f32;
        t.baseline = (y + px(17.0)) as f32;
        let how = t.clip(&format!("Next: {}", STEPS[i].how), 400, w - 2.0 * pad);
        t.draw(&how, x0 + pad, 400, ink.mix(bg, 0.4));
    }
    panel
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ops;
    use rhizo_core::NodeId;
    use rhizo_kit::Kits;
    use rhizo_lang::Editor;
    use serde_json::json;

    fn ticks(e: &Editor, seen: Seen) -> Vec<usize> {
        let done = progress(&start(), e.document(), seen);
        done.iter().enumerate().filter(|(_, d)| **d).map(|(i, _)| i + 1).collect()
    }

    /// Each step ticks off when its edit happens, not before, through the
    /// same edits the viewer's commands make.
    #[test]
    fn steps_tick_off_as_they_are_done() {
        let kits = Kits::builtin();
        let src = rhizo_lang::starters::starter("learn").unwrap();
        let mut e = Editor::open(src);
        assert!(is_tutorial(e.document()));
        assert!(e.diagnostics().is_empty(), "{:?}", e.diagnostics());
        assert_eq!(STEPS.len(), 10);
        assert_eq!(ticks(&e, Seen::default()), Vec::<usize>::new(), "nothing done yet");
        assert_eq!(count(&progress(&start(), e.document(), Seen::default())), "0 of 10");

        // 1. Tab on “My idea”: a child.
        let idea = NodeId::from("idea");
        let child = e.add_child(&idea, ops::NEW_LABEL).unwrap();
        let child = e.rename_new(&child, "Why").unwrap();
        assert_eq!(ticks(&e, Seen::default()), [1], "a lone child has no sibling yet");
        // 2. Enter on it: a sibling.
        let sib = e.add_after(&child, ops::NEW_LABEL).unwrap();
        e.rename_new(&sib, "How").unwrap();
        assert_eq!(ticks(&e, Seen::default()), [1, 2]);
        // 3. F2 on a node of the starter.
        e.rename(&idea, "My plan").unwrap();
        assert_eq!(ticks(&e, Seen::default()), [1, 2, 3]);
        // 4. Alt+drag: an edge.
        let (apples, pears) = (NodeId::from("apples"), NodeId::from("pears"));
        e.connect(&apples, &pears).unwrap();
        assert_eq!(ticks(&e, Seen::default()), [1, 2, 3, 4]);
        // 5. Both fruits selected, ⌘G.
        e.group(&[apples.clone(), pears.clone()], "Fruit bowl").unwrap();
        assert_eq!(ticks(&e, Seen::default()), [1, 2, 3, 4, 5]);
        // 6. The inspector types a node.
        ops::set_type(&mut e, &kits, std::slice::from_ref(&apples), Some("rhizo.goal")).unwrap();
        assert_eq!(ticks(&e, Seen::default()), [1, 2, 3, 4, 5, 6]);
        // 7. New frame, typed Environment, a cloud picked: only then.
        let f = e.add_frame(None, "New frame", None).unwrap();
        let env = ops::name_new_frame(&mut e, &f, "Production").unwrap();
        ops::set_frame_type(&mut e, &kits, &env, Some("infra.environment")).unwrap();
        assert!(!ticks(&e, Seen::default()).contains(&7), "a cloud and region are part of the step");
        let owner = ops::Owner::Frame(env.clone());
        ops::set_prop(&mut e, &owner, None, "cloud", Some(json!("aws"))).unwrap();
        assert!(!ticks(&e, Seen::default()).contains(&7), "the compile needs a region too");
        ops::set_prop(&mut e, &owner, None, "region", Some(json!("eu-central-1"))).unwrap();
        assert_eq!(ticks(&e, Seen::default()), [1, 2, 3, 4, 5, 6, 7]);
        assert!(e.diagnostics().iter().all(|d| !d.is_error()), "{:?}", e.diagnostics());
        // 8–10 are what the window saw.
        let all = Seen { compiled: true, pane: true, palette: true };
        assert_eq!(ticks(&e, all), (1..=10).collect::<Vec<_>>());
        assert_eq!(ticks(&e, Seen { pane: true, ..Seen::default() }), [1, 2, 3, 4, 5, 6, 7, 9]);
        assert_eq!(count(&progress(&start(), e.document(), all)), "10 of 10");

        // What the tutorial asks to compile does compile, as Diagram ▸ Compile specs… does.
        let dir = std::env::temp_dir().join(format!("rhizo-learn-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("learn.rhizo");
        std::fs::write(&path, e.source()).unwrap();
        let p = rhizo_project::Project::open(&path).unwrap();
        let compiled = p.compile(&kits);
        std::fs::remove_dir_all(&dir).ok();
        assert!(
            compiled.is_ok(),
            "{:?}",
            compiled.err().map(|f| f.into_iter().map(|f| f.diagnostic.message).collect::<Vec<_>>())
        );
    }

    /// A sibling at the top of a frame counts (Enter on “My idea”); a node
    /// alone in a new frame does not.
    #[test]
    fn a_sibling_is_beside_another_node() {
        let mut e = Editor::open(rhizo_lang::starters::starter("learn").unwrap());
        let n = e.add_after(&NodeId::from("idea"), "Other idea").unwrap();
        assert_eq!(e.document().node(&n).unwrap().parent, None);
        assert_eq!(ticks(&e, Seen::default()), [2]);
        let mut e = Editor::open(rhizo_lang::starters::starter("learn").unwrap());
        let f = e.add_frame(None, "Empty", None).unwrap();
        e.add_node(&f, "Alone").unwrap();
        assert!(ticks(&e, Seen::default()).is_empty(), "alone in its frame");
        assert!(!is_tutorial(&rhizo_lang::load("rhizo version=1\n", rhizo_lang::Syntax::Kdl).document));
    }

    /// Every step's keys come from the registry, and the card fits them.
    #[test]
    fn steps_name_registry_commands() {
        for s in STEPS {
            assert!(palette::command(s.action).is_some(), "{}", s.title);
        }
    }
}
