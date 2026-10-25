//! "Adopt node…": switching imported nodes to rhizo's code. The dry run
//! (shape checks and L1-adopt, OpenTofu for seconds to minutes) runs on a
//! worker thread while the status bar says how far it is; its report then
//! waits in a dialog, and only its Adopt button writes the file, as one
//! undoable edit.

use rhizo_app::adopt::{self, Button, Job, Outcome, Target};
use rhizo_app::inspector::Adoption;
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::Rect;
use rhizo_core::NodeId;
use rhizo_kit::infra as k;
use rhizo_lang::Editor;

use super::App;
use super::workers::{self, AdoptUpdate};

#[derive(Default)]
pub struct Adopting {
    pub job: Job,
    /// Where the report dialog's buttons were drawn.
    pub rects: Vec<(Rect, Button)>,
    pub hover: Option<Button>,
}

impl App {
    /// Starts a dry run adopting `target` in the open file.
    pub fn start_adopt(&mut self, target: Target) {
        self.start_adopt_allowing(target, Vec::new());
    }

    /// [`App::start_adopt`] with providers the person allowed for this run only.
    fn start_adopt_allowing(&mut self, target: Target, allow: Vec<String>) {
        let (Some(path), Some(editor)) = (self.session.path.clone(), &self.editor) else {
            return self.toast("Adopting works on .rhizo and .kdl files".into());
        };
        // The viewer saves every edit at once; typing waits until it parses.
        // Adopt reads the file as saved, so pending typing is refused, not saved.
        if self.pane_dirty() {
            return self.fail(
                "Adopt needs the saved file: the text pane has unsaved typing (⌘S saves it, ⌘K Revert text drops it)"
                    .into(),
            );
        }
        if let Target::Nodes(ids) = &target {
            for id in ids {
                if let Err(why) = rhizo_iac::adoptable(editor.document(), &NodeId::from(id.as_str())) {
                    return self.fail(why);
                }
            }
        }
        let source = editor.source().to_string();
        if std::fs::read_to_string(&path).ok().as_deref() != Some(source.as_str()) {
            return self.fail("The file on disk is not what the window shows yet: try again in a moment".into());
        }
        match self.adopting.job.start(target.clone(), path, source.clone()) {
            Ok(cancel) => workers::adopt(source, target, allow, cancel, self.proxy.clone()),
            Err(why) => self.toast(why),
        }
        self.redraw();
    }

    /// Stops the running dry run, or drops a waiting report.
    pub fn cancel_adopt(&mut self) {
        match &self.adopting.job {
            Job::Idle => self.toast("No adopt is running".into()),
            _ => self.adopting.job.cancel(),
        }
        self.redraw();
    }

    pub fn adopt_update(&mut self, u: AdoptUpdate) {
        match u {
            AdoptUpdate::Step(s) => self.adopting.job.step(s),
            AdoptUpdate::Done(result) => match self.adopting.job.finish(result) {
                Outcome::Review => self.adopting.hover = None,
                Outcome::Write { kdl, nodes, path, source } => {
                    let disk = std::fs::read_to_string(&path).ok();
                    if disk.as_deref() == Some(source.as_str()) && self.session.path.as_ref() == Some(&path) {
                        self.write_adopted(kdl, &nodes);
                    } else {
                        self.fail("The file changed while adopting: nothing was written; run Adopt again".into());
                    }
                }
                Outcome::Cancelled => self.toast("Adopt cancelled: nothing was written".into()),
                // Refusals list their reasons on lines of their own.
                Outcome::Failed(why) => self.fail(why.replace("\n  - ", " · ")),
            },
        }
        self.redraw();
    }

    /// The Adopt button: writes the reviewed diagram if the file is unchanged.
    /// After a provider refusal it is "Allow and adopt": the same dry run
    /// again, with those providers allowed for that run only.
    pub fn confirm_adopt(&mut self) {
        if let Some(r) = self.adopting.job.review().filter(|r| !r.allow.is_empty()) {
            let (target, allow) = (r.target.clone(), r.allow.clone());
            self.adopting.job.cancel();
            return self.start_adopt_allowing(target, allow);
        }
        let path = self.adopting.job.review().map(|r| r.path.clone());
        let disk = path.and_then(|p| std::fs::read_to_string(p).ok());
        match self.adopting.job.confirm(disk.as_deref()) {
            Ok(r) if self.session.path.as_ref() == Some(&r.path) => self.write_adopted(r.kdl, &r.nodes),
            Ok(_) => self.fail("The file was closed while adopting: nothing was written".into()),
            Err(why) => self.fail(why),
        }
        self.redraw();
    }

    /// Puts the adopted diagram in as one edit: saved, shown in the text,
    /// and undone by ⌘Z like any other.
    fn write_adopted(&mut self, kdl: String, nodes: &[String]) {
        if self
            .edit("Adopt", |e| {
                *e = Editor::open(kdl);
                Ok(())
            })
            .is_some()
        {
            let what = Target::Nodes(nodes.to_vec()).describe();
            self.toast(format!("Adopted {what}: its props are editable now (⌘Z undoes)"));
        }
    }

    /// Where the selected node stands with adopting, for the inspector.
    pub fn adoption(&self, id: &NodeId) -> Option<Adoption> {
        let doc = self.editor.as_ref()?.document();
        let n = doc.node(id)?;
        k::environment(doc, n).filter(|f| k::imported(f))?;
        if k::adopt::adopted(&n.props) {
            return Some(Adoption::Adopted);
        }
        if !n.props.contains_key("origin") {
            return None;
        }
        if matches!(self.adopting.job, Job::Running { .. }) {
            return Some(Adoption::Running);
        }
        Some(match rhizo_iac::adoptable(doc, id) {
            Ok(()) => Adoption::Ready,
            Err(why) => Adoption::Refused(why),
        })
    }

    /// The report dialog, in window pixels, while one waits.
    pub fn adopt_scene(&mut self) -> Option<Scene> {
        let (size, dpr, bg) = (self.size(), self.scale_factor(), self.background());
        self.adopting.job.review()?;
        self.fonts();
        self.mono();
        let (Some(ui), Some(mono)) = (self.fonts.as_ref(), self.mono.as_ref()) else { return None };
        let r = self.adopting.job.review()?;
        let mut s = Scene::new();
        self.adopting.rects = adopt::draw(&mut s, ui, mono, &mut self.glyphs, size, dpr, bg, r, self.adopting.hover);
        Some(s)
    }

    /// Scrolls the waiting report by `lines`.
    pub fn scroll_report(&mut self, lines: isize) {
        let (size, dpr) = (self.size(), self.scale_factor());
        let total = match (self.adopting.job.review(), self.mono.as_ref()) {
            (Some(r), Some(mono)) => {
                let (shown, _, total) = adopt::visible(r, mono, size, dpr);
                total.saturating_sub(shown.len())
            }
            _ => 0,
        };
        if let Some(r) = self.adopting.job.review_mut() {
            r.scroll = r.scroll.saturating_add_signed(lines).min(total);
            self.redraw();
        }
    }

    /// A press while the report waits: a button, else nothing.
    pub fn report_press(&mut self) {
        let p = self.cursor.to_point();
        match self.adopting.rects.iter().find(|(r, _)| r.contains(p)).map(|(_, b)| *b) {
            Some(Button::Adopt) => self.confirm_adopt(),
            Some(Button::Cancel) => self.cancel_adopt(),
            None => {}
        }
    }
}
