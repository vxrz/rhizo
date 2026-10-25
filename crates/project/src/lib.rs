//! Projects: several rhizo files that refer to each other.
//!
//! A project is a folder of `.rhizo` files (or one file plus everything it
//! imports). A file imports another with `import "model.rhizo" as=model` and
//! then refers to its node `order` as `model:order`, in edges or as a node
//! placed in its own frames. Nothing has to be declared first: plain names
//! still create local nodes. Declared things carry their type, props and
//! details everywhere they are referenced.
//!
//! [`Project::merged`] is the whole project as one document: references
//! replaced by what they point at, ids kept unless two files use the same
//! one (then `file.id`). Checking, compiling and the build brief work on it.
//! [`Project::display`] is one file as it should be drawn: references show
//! their target's label and look, with a dashed outline.
//!
//! [`Project::open`] is how every tool reads diagrams: a folder, a file and
//! what it imports, or a file in another format as a one-file project.
//! [`Project::check`] lists every problem, [`Project::drawable`] is what
//! renderers and exporters draw, [`Project::compile`] builds the bundle.

pub mod proposals;
pub mod samples;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::path::{Path, PathBuf};

use rhizo_compile::{Bundle, Origins};
use rhizo_core::{Document, EdgeId, FrameId, LineStyle, Node, NodeId};
use rhizo_interop::Format;
use rhizo_kit::{Finding, Kits, Subject};
use rhizo_lang::Diagnostic;
use rhizo_lang::ast::Span;
use rhizo_lang::lower::Lowered;

pub struct SourceFile {
    /// Path relative to the project root, forward slashes.
    pub rel: String,
    pub path: PathBuf,
    pub src: String,
    /// What the file is written in; other formats than rhizo are read-only.
    pub format: Format,
    pub doc: Document,
    /// Problems found while reading the file on its own.
    pub load: Vec<Diagnostic>,
    lowered: Option<Lowered>,
}

impl SourceFile {
    pub fn has_errors(&self) -> bool {
        self.load.iter().any(Diagnostic::is_error)
    }
}

pub struct Project {
    pub root: PathBuf,
    /// A served folder: imports may not leave it.
    confined: bool,
    /// Imports are read; not for text that is no file.
    follow: bool,
    pub files: Vec<SourceFile>,
    /// Problems with imports and references, per file.
    links: Vec<Vec<Diagnostic>>,
    pub merged: Document,
    /// Merged node id → (file, id in that file).
    node_origin: HashMap<NodeId, (usize, NodeId)>,
    /// Merged edge index → (file, edge index in that file).
    edge_origin: Vec<(usize, usize)>,
    /// (file, reference id) → merged id of what it points at.
    resolved: HashMap<(usize, NodeId), NodeId>,
}

const EXTENSIONS: [&str; 2] = ["rhizo", "kdl"];

/// Largest diagram file rhizo reads; anything bigger is refused rather
/// than read into memory (an import of `/dev/zero` must not hang).
pub const MAX_FILE: u64 = 16 * 1024 * 1024;

/// Reads a diagram file: a regular file (following links), at most
/// [`MAX_FILE`] bytes.
pub fn read_source(path: &Path) -> std::io::Result<String> {
    use std::io::{Error, ErrorKind, Read};
    let meta = std::fs::metadata(path)?;
    if !meta.is_file() {
        return Err(Error::new(ErrorKind::InvalidInput, "not a regular file"));
    }
    if meta.len() > MAX_FILE {
        return Err(Error::new(ErrorKind::InvalidInput, format!("larger than {} MB", MAX_FILE / 1024 / 1024)));
    }
    let mut s = String::new();
    std::fs::File::open(path)?.take(MAX_FILE + 1).read_to_string(&mut s)?;
    Ok(s)
}

/// Replaces `path` with `contents` so readers never see half a file:
/// writes a uniquely named temp file next to it (created exclusively, so a
/// planted symlink there makes it fail instead of being followed), keeps
/// the old file's permissions, then renames over it. A `path` that is a
/// symlink keeps being one: the file it points to is replaced.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let dir = target.parent().filter(|d| !d.as_os_str().is_empty()).unwrap_or(Path::new("."));
    let name = target.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
    let perms = std::fs::metadata(&target).ok().map(|m| m.permissions());
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    for attempt in 0..16 {
        let tmp = dir.join(format!(".{name}.{}-{nanos}-{attempt}.tmp", std::process::id()));
        let mut f = match std::fs::OpenOptions::new().write(true).create_new(true).open(&tmp) {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        };
        let done = f
            .write_all(contents)
            .and_then(|()| f.sync_all())
            .and_then(|()| perms.clone().map_or(Ok(()), |p| std::fs::set_permissions(&tmp, p)))
            .and_then(|()| std::fs::rename(&tmp, &target));
        if done.is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
        return done;
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free temp file name"))
}

impl Project {
    /// Every `.rhizo`/`.kdl` file under `dir` (hidden folders, `target`,
    /// `node_modules` and `build` skipped), plus what they import.
    pub fn load_dir(dir: &Path) -> Result<Project, String> {
        let mut found = Vec::new();
        scan(dir, dir, &mut found, &mut { usize::MAX }).map_err(|e| format!("cannot read {}: {e}", dir.display()))?;
        found.sort();
        if found.is_empty() {
            return Err(format!("no .rhizo files in {}", dir.display()));
        }
        Ok(Project::load(dir, found, true, None, true))
    }

    /// One file and everything it imports, transitively.
    pub fn load_file(file: &Path) -> Project {
        let (root, name) = split(file);
        Project::load(&root, vec![name], false, None, true)
    }

    /// A folder, or a file and everything it imports. A file in another
    /// format (Mermaid, Markdown, …) is a one-file project of its own.
    pub fn open(path: &Path) -> Result<Project, String> {
        if path.is_dir() {
            return Project::load_dir(path);
        }
        format_of(path).ok_or_else(|| format!("rhizo cannot read {}", path.display()))?;
        Ok(Project::load_file(path))
    }

    /// File `path` read as `format`, with `src` as its text (what an editor
    /// holds, saved or not), and what it imports from disk.
    pub fn with_text(path: &Path, src: String, format: Format) -> Project {
        let (root, name) = split(path);
        Project::load(&root, vec![name], false, Some((src, format)), true)
    }

    /// Text that is not a file, called `name`: its imports are not followed.
    pub fn open_text(name: &str, src: &str, format: Format) -> Project {
        Project::load(Path::new(""), vec![name.to_string()], false, Some((src.to_string(), format)), false)
    }

    /// `given`: the text and format of the first file, instead of reading it.
    fn load(
        root: &Path,
        start: Vec<String>,
        confined: bool,
        mut given: Option<(String, Format)>,
        follow: bool,
    ) -> Project {
        let mut files: Vec<SourceFile> = Vec::new();
        let mut links: Vec<Vec<Diagnostic>> = Vec::new();
        let mut index: HashMap<String, usize> = HashMap::new();
        let mut queue: VecDeque<String> = start.into_iter().collect();
        while let Some(rel) = queue.pop_front() {
            if index.contains_key(&rel) {
                continue;
            }
            let path = root.join(&rel);
            let (text, format) = match given.take() {
                Some((src, format)) => (Ok(src), format),
                // A served folder may hold a link to any file (`leak.rhizo -> ~/.token`).
                None if confined && outside(root, &path) => (
                    Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "it leads outside the project folder")),
                    Format::Rhizo,
                ),
                None => (read_source(&path), format_of(Path::new(&rel)).unwrap_or(Format::Rhizo)),
            };
            let (src, load, doc, lowered) = match text {
                Ok(src) => {
                    let (load, doc, lowered) = read(&src, format);
                    (src, load, doc, lowered)
                }
                Err(e) => (
                    String::new(),
                    vec![Diagnostic::error(format!("cannot read {rel}: {e}"), None)],
                    Document::default(),
                    None,
                ),
            };
            for target in doc.imports.values().filter(|_| follow) {
                let t = normalize(&rel, target);
                if importable(&t, confined).is_ok() {
                    queue.push_back(t);
                }
            }
            index.insert(rel.clone(), files.len());
            files.push(SourceFile { rel, path, src, format, doc, load, lowered });
            links.push(Vec::new());
        }
        let mut p = Project {
            root: root.to_path_buf(),
            confined,
            follow,
            files,
            links,
            merged: Document::default(),
            node_origin: HashMap::new(),
            edge_origin: Vec::new(),
            resolved: HashMap::new(),
        };
        p.merge(&index);
        p
    }

    pub fn is_single(&self) -> bool {
        self.files.len() == 1
    }

    /// The target of reference `id` in file `f`: (file, local id), following
    /// references to references. `Err(None)`: the imported file could not
    /// be read (reported at the import); `Ok(None)`: a cycle.
    fn target(
        &self,
        index: &HashMap<String, usize>,
        f: usize,
        id: &NodeId,
    ) -> Result<Option<(usize, NodeId)>, Option<Diagnostic>> {
        let (mut file, mut id) = (f, id.clone());
        for _ in 0..=self.files.len() {
            let doc = &self.files[file].doc;
            let Some(prefix) = rhizo_core::typed::curie_prefix(id.as_str()) else { return Ok(Some((file, id))) };
            let Some(path) = doc.imports.get(prefix) else { return Ok(Some((file, id))) };
            let rel = normalize(&self.files[file].rel, path);
            let Some(&t) = index.get(&rel).filter(|&&t| self.files[t].lowered.is_some()) else {
                return Err(None);
            };
            let local = NodeId(id.as_str()[prefix.len() + 1..].to_string());
            if !self.files[t].doc.nodes.contains_key(&local) {
                let mut d = Diagnostic::error(format!("`{id}`: {} has no node `{local}`", self.files[t].rel), None);
                let ids = self.files[t].doc.nodes.keys().map(NodeId::as_str);
                if let Some(close) = rhizo_lang::diag::suggest(local.as_str(), ids) {
                    d.help = Some(format!("did you mean `{prefix}:{close}`?"));
                }
                return Err(Some(d));
            }
            (file, id) = (t, local);
        }
        Ok(None) // a cycle of references
    }

    fn merge(&mut self, index: &HashMap<String, usize>) {
        let single = self.files.len() == 1;
        if single && (self.files[0].doc.imports.is_empty() || !self.follow) {
            // One file on its own: it is the project, exactly.
            let doc = &self.files[0].doc;
            self.node_origin = doc.nodes.keys().map(|id| (id.clone(), (0, id.clone()))).collect();
            self.edge_origin = (0..doc.edges.len()).map(|i| (0, i)).collect();
            self.merged = doc.clone();
            return;
        }
        // References and where they point.
        let mut targets: BTreeMap<(usize, NodeId), (usize, NodeId)> = BTreeMap::new();
        for f in 0..self.files.len() {
            for (alias, path) in &self.files[f].doc.imports.clone() {
                let rel = normalize(&self.files[f].rel, path);
                let span = find(&self.files[f].src, &format!("\"{path}\""));
                if let Err(why) = importable(&rel, self.confined) {
                    self.links[f].push(Diagnostic::error(format!("import `{alias}`: {why}"), span));
                    continue;
                }
                let missing = index.get(&rel).is_none_or(|&t| self.files[t].lowered.is_none());
                if missing {
                    self.links[f].push(Diagnostic::error(format!("import `{alias}`: cannot read `{path}`"), span));
                }
            }
            let refs: Vec<NodeId> = self.files[f]
                .doc
                .nodes
                .keys()
                .filter(|id| {
                    rhizo_core::typed::curie_prefix(id.as_str())
                        .is_some_and(|p| self.files[f].doc.imports.contains_key(p))
                })
                .cloned()
                .collect();
            for id in refs {
                match self.target(index, f, &id) {
                    Ok(Some(t)) => {
                        targets.insert((f, id), t);
                    }
                    Ok(None) => {
                        self.links[f].push(Diagnostic::error(format!("`{id}` refers to itself in a cycle"), None))
                    }
                    Err(Some(mut d)) => {
                        d.span = find(&self.files[f].src, id.as_str());
                        self.links[f].push(d);
                    }
                    Err(None) => {}
                }
            }
        }

        // Merged ids: kept unless two files use the same one.
        let stem = |f: usize| self.files[f].rel.trim_end_matches(".rhizo").trim_end_matches(".kdl").replace('/', ".");
        let mut count: HashMap<&str, usize> = HashMap::new();
        for (f, file) in self.files.iter().enumerate() {
            for id in file.doc.nodes.keys().filter(|id| !targets.contains_key(&(f, (*id).clone()))) {
                *count.entry(id.as_str()).or_default() += 1;
            }
        }
        // Decide every merged id up front, in file order, so a qualified id
        // (`a.x`) can never land on one a file already uses: then add `~2`…
        let mut table: HashMap<(usize, NodeId), NodeId> = HashMap::new();
        let mut taken: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut wanted: Vec<((usize, NodeId), String)> = Vec::new();
        for (f, file) in self.files.iter().enumerate() {
            for id in file.doc.nodes.keys().filter(|id| !targets.contains_key(&(f, (*id).clone()))) {
                let base = if single || count.get(id.as_str()).copied().unwrap_or(0) <= 1 {
                    id.to_string()
                } else {
                    format!("{}.{}", stem(f), id)
                };
                wanted.push(((f, id.clone()), base));
            }
        }
        // Unqualified ids first: they keep their name; qualified ones yield.
        wanted.sort_by_key(|((f, id), base)| (base.as_str() != id.as_str(), *f));
        for (key, base) in wanted {
            let mut name = base.clone();
            let mut n = 2;
            while !taken.insert(name.clone()) {
                name = format!("{base}~{n}");
                n += 1;
            }
            table.insert(key, NodeId(name));
        }
        let merged_id =
            |f: usize, id: &NodeId| -> NodeId { table.get(&(f, id.clone())).cloned().unwrap_or_else(|| id.clone()) };
        let resolve = |f: usize, id: &NodeId| -> NodeId {
            match targets.get(&(f, id.clone())) {
                Some((tf, tid)) => merged_id(*tf, tid),
                None => merged_id(f, id),
            }
        };

        let mut doc = Document::default();
        let mut frame_count: HashMap<&str, usize> = HashMap::new();
        for file in &self.files {
            for fr in &file.doc.frames {
                *frame_count.entry(fr.id.as_str()).or_default() += 1;
            }
        }
        let frame_id = |f: usize, id: &FrameId| -> FrameId {
            if single || frame_count.get(id.as_str()).copied().unwrap_or(0) <= 1 {
                id.clone()
            } else {
                FrameId(format!("{}.{}", stem(f), id))
            }
        };
        for (f, file) in self.files.iter().enumerate() {
            let d = &file.doc;
            if doc.title.is_none() && single {
                doc.title = d.title.clone();
            }
            for k in &d.kits {
                if !doc.kits.iter().any(|x| x.kit == k.kit) {
                    doc.kits.push(k.clone());
                }
            }
            for (k, v) in &d.prefixes {
                doc.prefixes.entry(k.clone()).or_insert_with(|| v.clone());
            }
            for (k, v) in &d.styles {
                doc.styles.entry(k.clone()).or_insert_with(|| v.clone());
            }
            if doc.base.is_none() {
                doc.base = d.base.clone();
            }
            if single {
                doc.theme = d.theme.clone();
                doc.imports = d.imports.clone();
            }
            for fr in &d.frames {
                let mut fr = fr.clone();
                fr.id = frame_id(f, &fr.id);
                fr.parent = fr.parent.as_ref().map(|p| frame_id(f, p));
                if fr.implicit && !single {
                    // Each file's loose nodes become a frame named after the file.
                    fr.implicit = false;
                    fr.label = Some(d.title.clone().unwrap_or_else(|| file.rel.clone()));
                }
                doc.frames.push(fr);
            }
            for n in d.nodes.values() {
                if targets.contains_key(&(f, n.id.clone())) {
                    continue;
                }
                let mut m: Node = n.clone();
                m.id = merged_id(f, &n.id);
                m.parent = n.parent.as_ref().map(|p| resolve(f, p));
                m.children.clear();
                m.frame = frame_id(f, &n.frame);
                self.node_origin.insert(m.id.clone(), (f, n.id.clone()));
                doc.nodes.insert(m.id.clone(), m);
            }
            for (i, e) in d.edges.iter().enumerate() {
                let mut m = e.clone();
                m.from = resolve(f, &e.from);
                m.to = resolve(f, &e.to);
                if !single {
                    m.id = e.id.as_ref().map(|id| EdgeId(format!("{}.{}", stem(f), id)));
                }
                self.edge_origin.push((f, i));
                doc.edges.push(m);
            }
        }
        // Children of a reference hang under what it points at; a parent that
        // did not make it into the merge leaves its children at the top.
        let lost: Vec<NodeId> = doc
            .nodes
            .values()
            .filter(|n| n.parent.as_ref().is_some_and(|p| !doc.nodes.contains_key(p)))
            .map(|n| n.id.clone())
            .collect();
        for id in lost {
            if let Some(n) = doc.nodes.get_mut(&id) {
                n.parent = None;
            }
        }
        doc.relink();
        for ((f, id), t) in &targets {
            self.resolved.insert((*f, id.clone()), merged_id(t.0, &t.1));
        }
        self.merged = doc;
    }

    /// File `f` as it should be drawn: references take their target's
    /// label, type and props, with a dashed outline.
    pub fn display(&self, f: usize) -> Document {
        let mut doc = self.files[f].doc.clone();
        for n in doc.nodes.values_mut() {
            let Some(target) = self.resolved.get(&(f, n.id.clone())).and_then(|t| self.merged.nodes.get(t)) else {
                continue;
            };
            if n.label == n.id.as_str() {
                n.label = target.label.clone();
            }
            if n.ty.is_none() {
                n.ty = target.ty.clone();
                n.props = target.props.clone();
            }
            if n.note.is_none() {
                n.note = target.note.clone();
            }
            if n.style.shape.is_none() {
                n.style.shape = target.style.shape;
            }
            if n.style.line.is_none() {
                n.style.line = Some(LineStyle::Dashed);
            }
        }
        doc
    }

    /// File `f` as every renderer and exporter draws it: [`Project::display`]
    /// plus the kits' default looks.
    pub fn drawable(&self, f: usize, kits: &Kits) -> Document {
        let mut doc = self.display(f);
        kits.apply_style_defaults(&mut doc);
        doc
    }

    /// Kit findings about file `f`, pointed at the statements they are about.
    pub fn locate(&self, f: usize, findings: Vec<Finding>) -> Vec<Diagnostic> {
        let file = &self.files[f];
        let subjects: Vec<Subject> = findings.iter().map(|x| x.subject.clone()).collect();
        let mut diags = rhizo_kit::locate(&file.src, file.format.syntax(), findings);
        if matches!(file.format, Format::Json | Format::Yaml) {
            for (d, subject) in diags.iter_mut().zip(subjects) {
                if let (None, Subject::Node(id)) = (d.span, subject) {
                    d.span = written(&file.src, &file.doc, &id);
                }
            }
        }
        diags
    }

    /// Every problem, per file: reading, imports and references, and the
    /// kits' rules (each file on its own, plus the edges that cross files).
    pub fn check(&self, kits: &Kits) -> Vec<Vec<Diagnostic>> {
        let mut out: Vec<Vec<Diagnostic>> = self.files.iter().map(|f| f.load.clone()).collect();
        for (f, l) in self.links.iter().enumerate() {
            out[f].extend(l.iter().cloned());
        }
        for (f, file) in self.files.iter().enumerate() {
            if file.has_errors() {
                continue;
            }
            out[f].extend(self.locate(f, rhizo_kit::check(&file.doc, kits)));
        }
        if !self.is_single() {
            // Edges that touch a reference only make sense with the target's type.
            let crossing: Vec<usize> = (0..self.merged.edges.len())
                .filter(|&i| {
                    let (f, local) = self.edge_origin[i];
                    let e = &self.files[f].doc.edges[local];
                    self.resolved.contains_key(&(f, e.from.clone())) || self.resolved.contains_key(&(f, e.to.clone()))
                })
                .collect();
            for finding in rhizo_kit::check(&self.merged, kits) {
                let Subject::Edge(i) = finding.subject else { continue };
                if !crossing.contains(&i) {
                    continue;
                }
                let (f, local) = self.edge_origin[i];
                let mut d = finding.diagnostic;
                d.span = self.files[f].lowered.as_ref().and_then(|l| l.edge_spans.get(local).copied().flatten());
                out[f].push(d);
            }
        }
        out
    }

    /// What reference `id` in file `f` points at: (file, id in that file).
    pub fn target_of(&self, f: usize, id: &NodeId) -> Option<(usize, &NodeId)> {
        self.resolved.get(&(f, id.clone())).and_then(|m| self.origin(m))
    }

    /// The file each merged edge was drawn in (parallel to `merged.edges`).
    pub fn edge_files(&self) -> Vec<usize> {
        self.edge_origin.iter().map(|(f, _)| *f).collect()
    }

    /// The file each merged node was defined in.
    pub fn node_files(&self) -> HashMap<NodeId, usize> {
        self.node_origin.iter().map(|(id, (f, _))| (id.clone(), *f)).collect()
    }

    /// Where merged node `id` was written: (file, id in that file).
    pub fn origin(&self, id: &NodeId) -> Option<(usize, &NodeId)> {
        self.node_origin.get(id).map(|(f, l)| (*f, l))
    }

    /// Name for the whole project: the file name for one file, else the folder.
    pub fn name(&self) -> String {
        if self.is_single() {
            return self.files[0].rel.clone();
        }
        // Canonical, so `.` or `shop/..` name the folder itself: the name keys the OpenTofu state.
        let root = std::fs::canonicalize(&self.root).unwrap_or_else(|_| self.root.clone());
        root.file_name().map_or("project".into(), |n| n.to_string_lossy().to_string())
    }

    /// All sources, for hashing: path and text of each file in order.
    pub fn sources(&self) -> String {
        if self.is_single() {
            return self.files[0].src.clone();
        }
        self.files.iter().map(|f| format!("{}\n{}\n", f.rel, f.src)).collect()
    }

    /// Which file each part of [`Project::merged`] came from; `None` for one file.
    pub fn origins(&self) -> Option<Origins> {
        (!self.is_single()).then(|| Origins {
            files: self.files.iter().map(|f| f.rel.clone()).collect(),
            edge_file: self.edge_files(),
            node_file: self.node_files(),
        })
    }

    /// The bundle for the whole project, or the kit findings that stop it.
    pub fn compile(&self, kits: &Kits) -> Result<Bundle, Vec<Finding>> {
        rhizo_compile::compile_with(&self.merged, kits, &self.name(), &self.sources(), self.origins().as_ref())
    }
}

/// Something in another file that uses a node: a reference to it, or an
/// edge that touches such a reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Usage {
    /// The file it is written in, relative to the project root.
    pub file: String,
    pub node_or_edge: Used,
    /// What to call it in a list: the reference's label, or `from → to`.
    pub label: String,
    /// Where it is written; `None` for a reference only an edge mentions.
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Used {
    /// The reference (`model:order`), by its id in that file.
    Node(NodeId),
    /// An index into that file's edges.
    Edge(usize),
}

impl Project {
    /// Where node `local_id` of file `file_rel` is used in other files: each
    /// reference to it, then the edges touching that reference. In file
    /// order, so the list is stable.
    pub fn usages(&self, file_rel: &str, local_id: &NodeId) -> Vec<Usage> {
        let Some(f) = self.files.iter().position(|x| x.rel == file_rel) else { return Vec::new() };
        let Some(target) = self.node_origin.iter().find(|(_, (of, l))| *of == f && l == local_id).map(|(m, _)| m)
        else {
            return Vec::new();
        };
        let mut refs: Vec<(usize, &NodeId)> =
            self.resolved.iter().filter(|(k, t)| *t == target && k.0 != f).map(|((rf, id), _)| (*rf, id)).collect();
        refs.sort();
        let mut out = Vec::new();
        for (rf, rid) in refs {
            let file = &self.files[rf];
            let spans = file.lowered.as_ref();
            let label = |id: &NodeId| self.label_in(rf, id);
            out.push(Usage {
                file: file.rel.clone(),
                node_or_edge: Used::Node(rid.clone()),
                label: label(rid),
                span: spans.and_then(|l| l.node_spans.get(rid).copied().flatten()),
            });
            for (i, e) in file.doc.edges.iter().enumerate().filter(|(_, e)| &e.from == rid || &e.to == rid) {
                let mut text = format!("{} → {}", label(&e.from), label(&e.to));
                if let Some(l) = e.label.as_deref().filter(|l| !l.is_empty()) {
                    text.push_str(&format!(" “{l}”"));
                }
                let span = spans.and_then(|l| l.edge_spans.get(i).copied().flatten());
                out.push(Usage { file: file.rel.clone(), node_or_edge: Used::Edge(i), label: text, span });
            }
        }
        out
    }

    /// Node `id`'s label as file `f` draws it (a bare reference shows its target's).
    fn label_in(&self, f: usize, id: &NodeId) -> String {
        let own = self.files[f].doc.nodes.get(id).map(|n| n.label.as_str()).unwrap_or(id.as_str());
        if own != id.as_str() {
            return own.to_string();
        }
        let target = self.resolved.get(&(f, id.clone())).and_then(|t| self.merged.nodes.get(t));
        target.map_or_else(|| own.to_string(), |t| t.label.clone())
    }
}

/// How far up [`root_of`] looks for the rest of a project.
pub const ROOT_LEVELS: usize = 5;

/// Most directory entries [`root_of`] reads per folder it looks at, so a
/// file deep in a huge tree still opens at once.
const ROOT_BUDGET: usize = 4000;

/// The project a file belongs to when only the file is opened: the nearest
/// folder that holds every file it imports and every file that imports it,
/// looking at most [`ROOT_LEVELS`] up and never above the home folder (when
/// the file is inside it). Without such files, the file's own folder.
pub fn root_of(file: &Path) -> PathBuf {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    root_within(file, home.as_deref())
}

fn root_within(file: &Path, home: Option<&Path>) -> PathBuf {
    let Some(dir) = file.parent() else { return PathBuf::new() };
    let inside_home = home.filter(|h| dir.starts_with(h));
    let ups: Vec<&Path> = dir
        .ancestors()
        .skip(1)
        .take(ROOT_LEVELS)
        .take_while(|a| inside_home.is_none_or(|h| a.starts_with(h)))
        .collect();
    // Imports reach up as far as their `..` go.
    let p = Project::load_file(file);
    let reach = p.files.iter().map(|f| f.rel.split('/').take_while(|s| *s == "..").count()).max().unwrap_or(0);
    let mut level = reach.min(ups.len());
    let at = |level: usize| if level == 0 { dir } else { ups[level - 1] };
    // Files further up, outside that folder, that import this one.
    let mut seen = std::collections::HashSet::new();
    for (i, up) in ups.iter().enumerate().skip(level) {
        let Ok(me) = file.strip_prefix(up) else { continue };
        let me = me.to_string_lossy().replace('\\', "/");
        let mut found = Vec::new();
        let _ = scan(up, up, &mut found, &mut { ROOT_BUDGET });
        let imports_me =
            found.iter().filter(|rel| !up.join(rel).starts_with(at(level)) && seen.insert(up.join(rel))).any(|rel| {
                read_source(&up.join(rel)).is_ok_and(|src| import_paths(&src).any(|path| normalize(rel, path) == me))
            });
        if imports_me {
            level = i + 1;
        }
    }
    at(level).to_path_buf()
}

/// The paths in a file's `import "…"` lines.
// shortcut: a line scan, not a parse (an `import` inside a string or a block comment counts); parse if that misleads.
fn import_paths(src: &str) -> impl Iterator<Item = &str> {
    src.lines().filter_map(|l| l.trim_start().strip_prefix("import")?.trim_start().strip_prefix('"')?.split('"').next())
}

/// What to write in file `from` to import file `to` (both relative to the
/// project root, forward slashes): `../model.rhizo` and the like.
pub fn import_path(from: &str, to: &str) -> String {
    let mut dir: Vec<&str> = from.split('/').collect();
    dir.pop();
    let to: Vec<&str> = to.split('/').collect();
    let (to_dir, name) = to.split_at(to.len() - 1);
    let common = dir.iter().zip(to_dir).take_while(|(a, b)| a == b).count();
    let mut parts = vec![".."; dir.len() - common];
    parts.extend(&to_dir[common..]);
    parts.extend(name);
    parts.join("/")
}

/// The format a file is read as, by its extension, if rhizo reads it.
fn format_of(path: &Path) -> Option<Format> {
    Format::from_path(path).filter(|f| f.can_import())
}

/// `path` as a project root and the file's name in it.
fn split(path: &Path) -> (PathBuf, String) {
    let root = path.parent().map(Path::to_path_buf).unwrap_or_default();
    (root, path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default())
}

/// `src` read as `format`: its problems, its document, and where its
/// statements are (`None` when it does not read).
fn read(src: &str, format: Format) -> (Vec<Diagnostic>, Document, Option<Lowered>) {
    let Some(syntax) = format.syntax() else {
        return match rhizo_interop::import(src, format) {
            Ok((doc, warnings)) => {
                let lowered = Lowered {
                    document: doc.clone(),
                    diagnostics: Vec::new(),
                    node_spans: HashMap::new(),
                    edge_spans: Vec::new(),
                };
                (warnings, doc, Some(lowered))
            }
            Err(errors) => (errors, Document::default(), None),
        };
    };
    let (ast, mut diags) = rhizo_lang::parse(src, syntax);
    if diags.iter().any(Diagnostic::is_error) {
        return (diags, Document::default(), None);
    }
    let lowered = rhizo_lang::lower::lower_with_sources(&ast);
    diags.extend(lowered.diagnostics.iter().cloned());
    (diags, lowered.document.clone(), Some(lowered))
}

/// Where node `id` is written in JSON or YAML text: its id, else its label,
/// as a whole word.
// shortcut: a text search, as the JSON/YAML readers keep no spans; the first match may be an edge naming the node. Track spans in rhizo-lang if that misleads.
fn written(src: &str, doc: &Document, id: &NodeId) -> Span {
    let word = |c: char| c.is_alphanumeric() || matches!(c, '-' | '_' | '.');
    let label = doc.nodes.get(id).map(|n| n.label.as_str()).filter(|l| !l.is_empty());
    [Some(id.as_str()), label].into_iter().flatten().find_map(|needle| {
        src.match_indices(needle)
            .find(|(i, _)| {
                !src[..*i].chars().next_back().is_some_and(word)
                    && !src[i + needle.len()..].chars().next().is_some_and(word)
            })
            .map(|(i, _)| (i, needle.len()))
    })
}

/// The span of the first occurrence of `needle` in `src`.
fn find(src: &str, needle: &str) -> Span {
    src.find(needle).map(|i| (i, needle.len()))
}

/// Only rhizo files, and in a served folder only files inside it.
fn importable(rel: &str, confined: bool) -> Result<(), String> {
    if !EXTENSIONS.iter().any(|e| rel.ends_with(&format!(".{e}"))) {
        return Err(format!("`{rel}` is not a .rhizo or .kdl file"));
    }
    // `:`: `C:/x.rhizo` is a drive on Windows, outside any folder.
    if confined && (rel.starts_with("..") || rel.contains(':')) {
        return Err(format!("`{rel}` is outside the project folder"));
    }
    Ok(())
}

/// True when `path` resolves, through links, to a place outside `root`.
fn outside(root: &Path, path: &Path) -> bool {
    match (root.canonicalize(), path.canonicalize()) {
        (Ok(r), Ok(p)) => !p.starts_with(r),
        _ => false,
    }
}

/// `path` as written in file `from` (both relative to the project root).
fn normalize(from: &str, path: &str) -> String {
    let mut parts: Vec<&str> = from.split('/').collect();
    parts.pop(); // the file name
    for seg in path.split(['/', '\\']) {
        match seg {
            "" | "." => {}
            ".." => {
                if parts.last().is_some_and(|p| *p != "..") {
                    parts.pop();
                } else {
                    parts.push("..");
                }
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// The `.rhizo`/`.kdl` files under `dir`, relative and sorted, as
/// [`Project::load_dir`] finds them.
pub fn list(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let _ = scan(dir, dir, &mut found, &mut { usize::MAX });
    found.sort();
    found
}

/// Collects rhizo files under `dir`, reading at most `budget` entries.
fn scan(root: &Path, dir: &Path, out: &mut Vec<String>, budget: &mut usize) -> std::io::Result<()> {
    for entry in std::fs::read_dir(dir)? {
        if *budget == 0 {
            return Ok(());
        }
        *budget -= 1;
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if entry.file_type()?.is_dir() {
            if !name.starts_with('.') && !matches!(name.as_str(), "target" | "node_modules" | "build") {
                scan(root, &path, out, budget)?;
            }
        } else if path.extension().is_some_and(|e| EXTENSIONS.iter().any(|x| e == *x))
            && let Ok(rel) = path.strip_prefix(root)
        {
            out.push(rel.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
