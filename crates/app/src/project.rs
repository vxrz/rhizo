//! The project around the open file: its root folder, its files, their
//! problems and typed things. Read off the UI thread, again whenever a file
//! under the root changes (polled by modification time).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use rhizo_core::NodeId;
use rhizo_kit::Kits;
use rhizo_lang::Diagnostic;
use rhizo_project::{Project, Usage, Used};

use crate::palette::Entry;
use crate::sidebar::Glyph;

/// What the window has open, for finding the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Opened {
    /// A folder opened as such: it is the root.
    Folder(PathBuf),
    /// A file: the root is found around it ([`rhizo_project::root_of`]).
    File(PathBuf),
}

/// What the project watcher follows, and its generation (bumped on every switch).
pub type Watched = Arc<Mutex<(u64, Option<Opened>)>>;

/// A project as read from disk.
pub struct Scan {
    pub generation: u64,
    pub root: PathBuf,
    /// Every rhizo file under the root, relative, sorted.
    pub files: Vec<String>,
    /// `None` when the root holds no rhizo files.
    pub project: Option<Project>,
    /// Problems of each of `project.files`.
    pub diagnostics: Vec<Vec<Diagnostic>>,
}

/// Something typed that a file defines: a node, or a frame.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Thing {
    /// The file, relative to the root.
    pub rel: String,
    pub id: String,
    pub label: String,
    pub ty: String,
    pub kind: Kind,
    /// A typed frame (a workflow, an environment): it cannot be referenced.
    pub frame: bool,
    /// Not part of another typed thing (a field of an entity is not).
    pub top: bool,
}

/// How the sidebar groups typed things, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    Entity,
    Enum,
    Operation,
    Flow,
    Environment,
    Goal,
    Decision,
    Other,
}

impl Kind {
    /// By the type's last part: `data.entity` is an entity, `flow.workflow` a flow.
    pub fn of(ty: &str) -> Kind {
        match ty.rsplit('.').next().unwrap_or(ty) {
            "entity" => Kind::Entity,
            "enum" => Kind::Enum,
            "operation" => Kind::Operation,
            "workflow" | "flow" => Kind::Flow,
            "environment" => Kind::Environment,
            "goal" => Kind::Goal,
            "decision" => Kind::Decision,
            _ => Kind::Other,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Kind::Entity => "Entities",
            Kind::Enum => "Enums",
            Kind::Operation => "Operations",
            Kind::Flow => "Flows",
            Kind::Environment => "Environments",
            Kind::Goal => "Goals",
            Kind::Decision => "Decisions",
            Kind::Other => "Other",
        }
    }
}

/// One problem somewhere in the project.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub rel: String,
    pub error: bool,
    pub message: String,
    /// 1-based line and column, when the problem has a place.
    pub at: Option<(usize, usize)>,
    /// Byte offset in the file, when the problem has a place.
    pub offset: Option<usize>,
}

impl Thing {
    /// Its row in the Insert reference list: label, then type and file.
    pub fn entry(&self) -> Entry {
        let name = if self.label.is_empty() { self.id.clone() } else { self.label.clone() };
        Entry { name, detail: format!("{} · {}", self.ty, self.rel), glyph: Some(Glyph::Kind(self.kind)) }
    }
}

impl Problem {
    /// Its row in the Problems list: the message, then where.
    pub fn entry(&self) -> Entry {
        let glyph = Some(if self.error { Glyph::Error } else { Glyph::Warning });
        Entry { name: self.message.clone(), detail: self.place(), glyph }
    }

    /// `api.rhizo:10`, or just the file.
    pub fn place(&self) -> String {
        match self.at {
            Some((line, _)) => format!("{}:{line}", self.rel),
            None => self.rel.clone(),
        }
    }
}

/// Where a pick from a list goes: a file, and in it a node, an edge or a place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Spot {
    File,
    Node(NodeId),
    Edge(usize),
    Offset(usize),
}

impl Scan {
    /// Reads the project around `opened`.
    pub fn read(generation: u64, opened: &Opened, kits: &Kits) -> Scan {
        let root = match opened {
            Opened::Folder(dir) => dir.clone(),
            Opened::File(file) => rhizo_project::root_of(file),
        };
        Scan::at(generation, root, kits)
    }

    fn at(generation: u64, root: PathBuf, kits: &Kits) -> Scan {
        let files = rhizo_project::list(&root);
        let project = (!files.is_empty()).then(|| Project::load_dir(&root).ok()).flatten();
        let diagnostics = project.as_ref().map(|p| p.check(kits)).unwrap_or_default();
        Scan { generation, root, files, project, diagnostics }
    }

    /// `path` relative to the root, when it is under it.
    pub fn rel(&self, path: &Path) -> Option<String> {
        let rel = path.strip_prefix(&self.root).ok()?.to_string_lossy().replace('\\', "/");
        self.files.contains(&rel).then_some(rel)
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    /// Every problem, file by file, errors first, then by line.
    pub fn problems(&self) -> Vec<Problem> {
        let Some(p) = &self.project else { return Vec::new() };
        let mut out = Vec::new();
        for (file, diags) in p.files.iter().zip(&self.diagnostics) {
            let mut list: Vec<Problem> = diags
                .iter()
                .map(|d| Problem {
                    rel: file.rel.clone(),
                    error: d.is_error(),
                    message: d.message.clone(),
                    at: d.line_col(&file.src),
                    offset: d.span.map(|s| s.0).filter(|&o| o <= file.src.len()),
                })
                .collect();
            list.sort_by_key(|p| (!p.error, p.at));
            out.extend(list);
        }
        out
    }

    /// Errors and warnings in `rel` (`None`: the whole project).
    pub fn counts(&self, rel: Option<&str>) -> (usize, usize) {
        let Some(p) = &self.project else { return (0, 0) };
        let diags = p.files.iter().zip(&self.diagnostics).filter(|(f, _)| rel.is_none_or(|r| f.rel == r));
        let all: Vec<&Diagnostic> = diags.flat_map(|(_, d)| d).collect();
        let errors = all.iter().filter(|d| d.is_error()).count();
        (errors, all.len() - errors)
    }

    /// The typed things each file defines, in file order and then as written.
    /// References to other files' things are not things of their own.
    pub fn things(&self) -> Vec<Thing> {
        let Some(p) = &self.project else { return Vec::new() };
        let mut out = Vec::new();
        for file in &p.files {
            let doc = &file.doc;
            for f in &doc.frames {
                if let Some(ty) = &f.ty {
                    let label = f.label.clone().unwrap_or_else(|| f.id.to_string());
                    let ty = ty.as_str().to_string();
                    let (rel, id) = (file.rel.clone(), f.id.to_string());
                    out.push(Thing { rel, id, label, kind: Kind::of(&ty), ty, frame: true, top: true });
                }
            }
            let reference =
                |id: &NodeId| rhizo_core::typed::curie_prefix(id.as_str()).is_some_and(|p| doc.imports.contains_key(p));
            for n in doc.nodes.values().filter(|n| !reference(&n.id)) {
                let Some(ty) = &n.ty else { continue };
                let ty = ty.as_str().to_string();
                let top = n.parent.as_ref().and_then(|p| doc.nodes.get(p)).is_none_or(|p| p.ty.is_none());
                out.push(Thing {
                    rel: file.rel.clone(),
                    id: n.id.to_string(),
                    label: n.label.clone(),
                    kind: Kind::of(&ty),
                    ty,
                    frame: false,
                    top,
                });
            }
        }
        out
    }

    /// Where node `id` of `rel` is used in other files.
    pub fn usages(&self, rel: &str, id: &NodeId) -> Vec<Usage> {
        self.project.as_ref().map(|p| p.usages(rel, id)).unwrap_or_default()
    }

    /// How many other files use node `id` of `rel`.
    pub fn used_in(&self, rel: &str, id: &NodeId) -> usize {
        let mut files: Vec<String> = self.usages(rel, id).into_iter().map(|u| u.file).collect();
        files.dedup();
        files.len()
    }

    /// The 1-based line of byte `offset` in file `rel`.
    pub fn line(&self, rel: &str, offset: usize) -> Option<usize> {
        let f = self.project.as_ref()?.files.iter().find(|f| f.rel == rel)?;
        Some(f.src.get(..offset)?.matches('\n').count() + 1)
    }

    /// A usage's row in the Find usages list: what it is, then where.
    pub fn usage_entry(&self, u: &Usage) -> Entry {
        let line = u.span.and_then(|(at, _)| self.line(&u.file, at));
        let glyph = Some(if matches!(u.node_or_edge, Used::Edge(_)) { Glyph::Edge } else { Glyph::Reference });
        Entry {
            name: u.label.clone(),
            detail: line.map_or_else(|| u.file.clone(), |l| format!("{}:{l}", u.file)),
            glyph,
        }
    }

    /// A usage as a place to jump to.
    pub fn spot(u: &Usage) -> Spot {
        match &u.node_or_edge {
            Used::Node(id) => Spot::Node(id.clone()),
            Used::Edge(i) => Spot::Edge(*i),
        }
    }
}

/// How often the watcher looks at the files.
const POLL: Duration = Duration::from_millis(400);

/// Follows the project around what `watched` names: reads it, then again
/// whenever a file under the root appears, goes or changes. `send` gets
/// each reading and returns false once nobody listens.
pub fn watch(watched: Watched, poke: Receiver<()>, mut send: impl FnMut(Scan) -> bool + Send + 'static) {
    std::thread::spawn(move || {
        let kits = Kits::builtin();
        let mut seen: Option<u64> = None;
        let mut root: Option<PathBuf> = None;
        let mut last: Option<Vec<(String, Option<SystemTime>)>> = None;
        loop {
            let (generation, opened) = watched.lock().expect("UI alive").clone();
            if seen != Some(generation) {
                seen = Some(generation);
                last = None;
                root = opened.as_ref().map(|o| match o {
                    Opened::Folder(dir) => dir.clone(),
                    Opened::File(file) => rhizo_project::root_of(file),
                });
                // Nothing open: say so once, so the sidebar empties.
                let none = || Scan {
                    generation,
                    root: PathBuf::new(),
                    files: Vec::new(),
                    project: None,
                    diagnostics: Vec::new(),
                };
                if root.is_none() && !send(none()) {
                    return;
                }
            }
            if let Some(root) = &root {
                let stamp: Vec<(String, Option<SystemTime>)> = rhizo_project::list(root)
                    .into_iter()
                    .map(|rel| {
                        let t = std::fs::metadata(root.join(&rel)).and_then(|m| m.modified()).ok();
                        (rel, t)
                    })
                    .collect();
                if last.as_ref() != Some(&stamp) {
                    last = Some(stamp);
                    if !send(Scan::at(generation, root.clone(), &kits)) {
                        return;
                    }
                }
            }
            // A save wakes us at once; otherwise poll.
            if poke.recv_timeout(POLL) == Err(RecvTimeoutError::Disconnected) {
                return;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shop() -> Scan {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop");
        Scan::read(1, &Opened::Folder(root), &Kits::builtin())
    }

    #[test]
    fn kinds_by_type() {
        assert_eq!(Kind::of("data.entity"), Kind::Entity);
        assert_eq!(Kind::of("flow.workflow"), Kind::Flow);
        assert_eq!(Kind::of("rhizo.flow"), Kind::Flow);
        assert_eq!(Kind::of("infra.environment"), Kind::Environment);
        assert_eq!(Kind::of("rhizo.decision"), Kind::Decision);
        assert_eq!(Kind::of("aws.lambda.function"), Kind::Other);
        assert!(Kind::Entity < Kind::Other);
    }

    #[test]
    fn the_shop_project() {
        let s = shop();
        assert_eq!(s.files.len(), 6);
        assert_eq!(s.counts(None), (0, 0), "{:?}", s.problems());
        let things = s.things();
        let of = |rel: &str| -> Vec<(&str, Kind, bool)> {
            things.iter().filter(|t| t.rel == rel && t.top).map(|t| (t.id.as_str(), t.kind, t.frame)).collect()
        };
        assert_eq!(
            of("model.rhizo"),
            [("customer", Kind::Entity, false), ("order", Kind::Entity, false), ("status", Kind::Enum, false)],
            "fields belong to their entity"
        );
        assert_eq!(of("intent.rhizo")[0], ("goal", Kind::Goal, false));
        assert!(of("api.rhizo").contains(&("place-order", Kind::Operation, false)));
        assert!(of("api.rhizo").contains(&("shop-api", Kind::Other, true)), "typed frames count");
        assert!(of("flows/checkout.rhizo").contains(&("checkout", Kind::Flow, true)));
        assert!(!things.iter().any(|t| t.id.contains(':')), "references are not things");
        assert!(things.iter().any(|t| t.id == "customer-email" && !t.top));

        assert_eq!(s.used_in("model.rhizo", &NodeId::from("order")), 2, "api and infra");
        assert_eq!(s.rel(&s.root.join("flows/checkout.rhizo")).as_deref(), Some("flows/checkout.rhizo"));
        assert_eq!(s.rel(Path::new("/elsewhere/x.rhizo")), None);

        // Picker rows: things to refer to, and where `order` is used.
        let order = things.iter().find(|t| t.id == "order").unwrap();
        let e = order.entry();
        assert_eq!((e.name.as_str(), e.detail.as_str()), ("Order", "data.entity · model.rhizo"));
        assert_eq!(e.glyph, Some(Glyph::Kind(Kind::Entity)));
        let rows: Vec<(String, String, Option<Glyph>)> = s
            .usages("model.rhizo", &NodeId::from("order"))
            .iter()
            .map(|u| s.usage_entry(u))
            .map(|e| (e.name, e.detail, e.glyph))
            .collect();
        assert_eq!(rows[0], ("Order".into(), "api.rhizo".into(), Some(Glyph::Reference)), "made by an edge: no line");
        assert_eq!(rows[1], ("List orders → Order “200”".into(), "api.rhizo:17".into(), Some(Glyph::Edge)));
        assert!(rows.iter().any(|r| r.1.starts_with("infra.rhizo:")));
    }

    #[test]
    fn problems_with_places() {
        let dir = std::env::temp_dir().join(format!("rhizo-app-problems-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("model.rhizo"), "rhizo version=2\nnode \"Order\" id=order\n").unwrap();
        std::fs::write(
            dir.join("flow.rhizo"),
            "rhizo version=2\nimport \"model.rhizo\" as=model\nedge \"Pay\" model:ordr\n",
        )
        .unwrap();
        std::fs::write(dir.join("other.rhizo"), "rhizo version=1\n- \"X\" shap=box\n").unwrap();
        let s = Scan::read(7, &Opened::File(dir.join("flow.rhizo")), &Kits::builtin());
        assert_eq!(s.root, dir);
        assert_eq!(s.generation, 7);
        let p = s.problems();
        assert_eq!(p.len(), 2, "{p:#?}");
        assert!(p.iter().all(|p| p.error));
        assert_eq!(p[0].place(), "flow.rhizo:3");
        assert_eq!(p[1].place(), "other.rhizo:2");
        let e = p[0].entry();
        assert_eq!((e.detail.as_str(), e.glyph), ("flow.rhizo:3", Some(Glyph::Error)));
        assert!(e.name.contains("model:ordr"));
        let o = p[0].offset.unwrap();
        assert_eq!(s.line("flow.rhizo", o), Some(3));
        assert_eq!(s.counts(Some("flow.rhizo")), (1, 0));
        assert_eq!(s.counts(Some("model.rhizo")), (0, 0));
        assert_eq!(s.counts(None), (2, 0));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
