//! What the MCP tools do, without the protocol: each call reads the document
//! fresh from disk, so answers always match the file. Nothing here writes the
//! document or the bundle (design doc §7.1, rules 2 and 3); proposals go to
//! `.rhizo/proposals/` for a person to apply.

use std::collections::{BTreeMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};

use rhizo_core::{Document, Edge, Node, NodeId};
use rhizo_interop::Format;
use rhizo_kit::Kits;
use rhizo_lang::{Diagnostic, Severity};
use rhizo_project::Project;
use serde_json::{Value, json};

#[derive(Debug)]
pub enum Rendered {
    Svg(String),
    Png(Vec<u8>),
}

pub struct Workspace {
    pub path: PathBuf,
    kits: Kits,
    /// Providers the person allowed when starting the server (`rhizo mcp
    /// --allow-provider`), besides rhizo's allow-list. Never set by an agent.
    pub allow_providers: Vec<String>,
}

/// A node's summary, with the file it comes from when there are several.
fn summary_in(p: &Project, n: &Node) -> Value {
    let mut v = summary(n);
    if let Some((f, _)) = p.origin(&n.id).filter(|_| !p.is_single()) {
        merge(&mut v, json!({ "file": p.files[f].rel }));
    }
    v
}

/// Largest diagram `render` lays out in one request.
const MAX_RENDER_NODES: usize = 3000;
const MAX_RENDER_EDGES: usize = 12000;

/// Most nodes a listing returns, so a huge map cannot flood the context.
const LIST_LIMIT: usize = 500;

impl Workspace {
    pub fn new(path: PathBuf) -> Self {
        Workspace { path, kits: Kits::builtin(), allow_providers: Vec::new() }
    }

    fn name(&self) -> String {
        self.path.file_name().map_or("document".into(), |n| n.to_string_lossy().to_string())
    }

    /// The folder or file, or an error naming its first problem.
    fn load(&self) -> Result<Project, String> {
        let p = Project::open(&self.path)?;
        for f in &p.files {
            if let Some(d) = f.load.iter().find(|d| d.is_error()) {
                // A file in another format is "the document", as it was before projects.
                let what = if f.format == Format::Rhizo { format!("{} has", f.rel) } else { "the document has".into() };
                return Err(format!("{what} errors; call `check` for all of them. First: {}", describe(&f.src, d)));
            }
        }
        Ok(p)
    }

    pub fn overview(&self) -> Result<Value, String> {
        let p = self.load()?;
        let doc = &p.merged;
        let frames: Vec<Value> = doc
            .frames
            .iter()
            .map(|f| {
                json!({
                    "id": f.id.as_str(),
                    "label": f.label,
                    "type": f.ty.as_ref().map(|t| t.as_str()),
                    "layout": f.layout.name(),
                    "nodes": doc.nodes.values().filter(|n| n.frame == f.id).count(),
                })
            })
            .collect();
        let mut types: BTreeMap<&str, usize> = BTreeMap::new();
        for ty in doc.nodes.values().filter_map(|n| n.ty.as_ref()) {
            *types.entry(ty.as_str()).or_default() += 1;
        }
        let kits: Vec<Value> = doc.kits.iter().map(|k| json!({ "kit": k.kit, "version": k.version })).collect();
        let files: Vec<&str> =
            if p.is_single() { Vec::new() } else { p.files.iter().map(|f| f.rel.as_str()).collect() };
        Ok(json!({
            "file": self.path.display().to_string(),
            "files": files,
            "title": doc.title,
            "kits": kits,
            "frames": frames,
            "nodes": doc.nodes.len(),
            "edges": doc.edges.len(),
            "types": types,
        }))
    }

    pub fn list_nodes(
        &self,
        ty: Option<&str>,
        frame: Option<&str>,
        tag: Option<&str>,
        query: Option<&str>,
    ) -> Result<Value, String> {
        let p = self.load()?;
        let doc = &p.merged;
        let query = query.map(str::to_lowercase);
        let matches: Vec<&Node> = doc
            .nodes
            .values()
            .filter(|n| ty.is_none_or(|t| type_matches(n, t)))
            .filter(|n| frame.is_none_or(|f| n.frame.as_str() == f))
            .filter(|n| tag.is_none_or(|t| n.tags.iter().any(|x| x == t)))
            .filter(|n| {
                query.as_ref().is_none_or(|q| n.label.to_lowercase().contains(q) || n.id.as_str().contains(q.as_str()))
            })
            .collect();
        let shown: Vec<Value> = matches.iter().take(LIST_LIMIT).map(|n| summary_in(&p, n)).collect();
        Ok(json!({ "total": matches.len(), "nodes": shown }))
    }

    pub fn get_node(&self, id: &str) -> Result<Value, String> {
        let p = self.load()?;
        let doc = &p.merged;
        let n = node(doc, id)?;
        let def = n.ty.as_ref().and_then(|t| self.kits.find_type(t.as_str()));
        let mut ports: Vec<Value> = def
            .map(|d| d.ports.iter().map(|p| json!({ "port": p.port, "default": p.default, "from": "kit" })).collect())
            .unwrap_or_default();
        ports.extend(n.ports.iter().map(|p| json!({ "port": p, "from": "node" })));
        let edges = |pick: fn(&Edge) -> &NodeId| -> Vec<Value> {
            doc.edges.iter().filter(|e| pick(e) == &n.id).map(edge).collect()
        };
        let mut v = summary_in(&p, n);
        let extra = json!({
            "props": n.props,
            "kit_type": def.map(|d| json!({ "label": d.label, "description": d.description, "frame": d.frame })),
            "ports": ports,
            "note": n.note,
            "link": n.link,
            "tags": n.tags,
            "accept": n.accept,
            "prov": n.prov,
            "children": n.children.iter().map(NodeId::as_str).collect::<Vec<_>>(),
            "outgoing": edges(|e| &e.from),
            "incoming": edges(|e| &e.to),
        });
        merge(&mut v, extra);
        Ok(v)
    }

    pub fn neighbors(
        &self,
        id: &str,
        kind: Option<&str>,
        direction: Option<&str>,
        depth: Option<u32>,
    ) -> Result<Value, String> {
        let p = self.load()?;
        let doc = &p.merged;
        let start = node(doc, id)?;
        let (out, inc) = match direction.unwrap_or("both") {
            "out" => (true, false),
            "in" => (false, true),
            "both" => (true, true),
            other => return Err(format!("direction `{other}`: use in, out or both")),
        };
        let depth = depth.unwrap_or(1).clamp(1, 5);
        let kind_ok = |e: &Edge| match kind {
            None => true,
            Some("plain") => e.kind.is_none(),
            Some(k) => e.kind.is_some_and(|x| x.name() == k),
        };
        let mut seen: HashSet<&NodeId> = HashSet::from([&start.id]);
        let mut queue = VecDeque::from([(&start.id, 0)]);
        let (mut nodes, mut edges, mut used) = (Vec::new(), Vec::new(), HashSet::new());
        while let Some((cur, d)) = queue.pop_front() {
            if d == depth {
                continue;
            }
            for (i, e) in doc.edges.iter().enumerate().filter(|(_, e)| kind_ok(e)) {
                let next = if out && &e.from == cur {
                    &e.to
                } else if inc && &e.to == cur {
                    &e.from
                } else {
                    continue;
                };
                if used.insert(i) {
                    edges.push(edge(e));
                }
                if seen.insert(next) {
                    if let Some(n) = doc.nodes.get(next) {
                        let mut s = summary_in(&p, n);
                        merge(&mut s, json!({ "distance": d + 1 }));
                        nodes.push(s);
                    }
                    queue.push_back((next, d + 1));
                }
            }
        }
        Ok(json!({ "node": id, "nodes": nodes, "edges": edges }))
    }

    /// Where a node is used in other files of the project: references to it,
    /// and the edges that touch them. `id` is a node id as `list_nodes` gives
    /// it, or the id inside `file`. A single file is looked at within the
    /// folder it belongs to, so files importing it count.
    pub fn usages(&self, id: &str, file: Option<&str>) -> Result<Value, String> {
        let (p, own) = if self.path.is_dir() {
            (Project::load_dir(&self.path)?, None)
        } else {
            let root = rhizo_project::root_of(&self.path);
            let rel = self.path.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).ok();
            (Project::load_dir(&root)?, rel)
        };
        let (rel, local) = match file.map(str::to_string).or(own) {
            Some(rel) => (rel, NodeId(id.to_string())),
            None => match p.origin(&NodeId(id.to_string())) {
                Some((f, local)) => (p.files[f].rel.clone(), local.clone()),
                None => return Err(format!("no node `{id}`; `list_nodes` gives the ids")),
            },
        };
        let f = p.files.iter().find(|f| f.rel == rel).ok_or_else(|| format!("no file `{rel}` in the project"))?;
        if !f.doc.nodes.contains_key(&local) {
            return Err(format!("{rel} has no node `{local}`"));
        }
        let list: Vec<Value> = p
            .usages(&rel, &local)
            .iter()
            .map(|u| {
                let src = p.files.iter().find(|f| f.rel == u.file).map_or("", |f| f.src.as_str());
                let line = u.span.map(|(at, _)| src[..at].matches('\n').count() + 1);
                let (kind, what) = match &u.node_or_edge {
                    rhizo_project::Used::Node(id) => ("reference", json!(id.as_str())),
                    rhizo_project::Used::Edge(i) => ("edge", json!(i)),
                };
                json!({ "file": u.file, "kind": kind, "id": what, "label": u.label, "line": line })
            })
            .collect();
        let mut files: Vec<&str> = list.iter().filter_map(|u| u["file"].as_str()).collect();
        files.dedup();
        Ok(json!({ "node": local.as_str(), "file": rel, "files": files, "usages": list }))
    }

    /// Every problem: syntax, structure, imports, references and kit rules,
    /// with file, line and column.
    pub fn check(&self) -> Result<Value, String> {
        let p = Project::open(&self.path)?;
        let mut list = Vec::new();
        let mut errors = 0;
        for (f, diags) in p.check(&self.kits).iter().enumerate() {
            for d in diags {
                errors += usize::from(d.is_error());
                let mut v = diagnostic(&p.files[f].src, d);
                if !p.is_single() {
                    merge(&mut v, json!({ "file": p.files[f].rel }));
                }
                list.push(v);
            }
        }
        Ok(json!({ "ok": errors == 0, "errors": errors, "warnings": list.len() - errors, "diagnostics": list }))
    }

    /// Like `check`, for rhizo text that is not saved anywhere.
    pub fn validate(&self, source: &str) -> Value {
        self.report(source)
    }

    fn report(&self, src: &str) -> Value {
        let diags = Project::open_text(&self.name(), src, Format::Rhizo).check(&self.kits).swap_remove(0);
        let errors = diags.iter().filter(|d| d.is_error()).count();
        let list: Vec<Value> = diags.iter().map(|d| diagnostic(src, d)).collect();
        json!({ "ok": errors == 0, "errors": errors, "warnings": diags.len() - errors, "diagnostics": list })
    }

    pub fn guide(&self) -> String {
        self.kits.guide()
    }

    /// The built-in samples (`rhizo samples`): the list, or one sample's files.
    pub fn samples(&self, id: Option<&str>) -> Result<Value, String> {
        use rhizo_project::samples::{SAMPLES, find};
        let Some(id) = id else {
            let list: Vec<Value> = SAMPLES
                .iter()
                .map(|s| json!({ "id": s.id, "title": s.title, "about": s.blurb, "group": s.group, "files": s.files.len() }))
                .collect();
            return Ok(json!({ "samples": list }));
        };
        let s = find(id).ok_or_else(|| format!("no sample `{id}`: call samples without an id for the list"))?;
        let files: Vec<Value> = s.files.iter().map(|(path, text)| json!({ "path": path, "text": text })).collect();
        Ok(json!({ "id": s.id, "title": s.title, "files": files }))
    }

    /// The build brief as Markdown, or the task graph as JSON.
    pub fn brief(&self, json: bool) -> Result<String, String> {
        let mut bundle = self.load()?.compile(&self.kits).map_err(|findings| {
            let msgs: Vec<String> = findings.iter().map(|f| f.diagnostic.message.clone()).collect();
            format!("fix these first: {}", msgs.join("; "))
        })?;
        let key = if json { rhizo_compile::TASKS } else { rhizo_compile::AGENTS };
        Ok(bundle.files.remove(key).map(|f| f.contents).unwrap_or_default())
    }

    /// The diagram as SVG text or PNG bytes, as `rhizo render` draws it.
    pub fn render(&self, png: bool, file: Option<&str>) -> Result<Rendered, String> {
        let p = self.load()?;
        let all = || p.files.iter().map(|f| f.rel.as_str()).collect::<Vec<_>>().join(", ");
        let f = match file {
            Some(rel) => {
                p.files.iter().position(|f| f.rel == rel).ok_or_else(|| format!("no file `{rel}`; files: {}", all()))?
            }
            None if self.path.is_dir() => {
                return Err(format!("this is a project; say which `file` to draw: {}", all()));
            }
            None => 0,
        };
        let doc = p.drawable(f, &self.kits);
        // Layout time grows fast with size; keep one request bounded.
        if doc.nodes.len() > MAX_RENDER_NODES || doc.edges.len() > MAX_RENDER_EDGES {
            return Err(format!(
                "too large to render here ({} nodes, {} edges; the limit is {MAX_RENDER_NODES} and {MAX_RENDER_EDGES}). \
                 Use `rhizo render` on the command line.",
                doc.nodes.len(),
                doc.edges.len()
            ));
        }
        let theme = rhizo_render::theme_for(&doc, None);
        // shortcut: loads system fonts on every call (~0.3 s); cache them if renders get frequent.
        let fonts = rhizo_render::Fonts::system(theme.font_family);
        let diagram = rhizo_render::layout(&doc, &theme, &fonts);
        let svg = rhizo_render::to_svg(&diagram, &Default::default());
        if !png {
            return Ok(Rendered::Svg(svg));
        }
        // Keep images a size a model can take in: at most ~2000 px on the long side.
        // At most ~2000 px on the long side and 8 megapixels: a size a model
        // can take in, and bounded memory.
        let (w, h) = (diagram.bounds.width.max(1.0), diagram.bounds.height.max(1.0));
        let scale = (2000.0 / w.max(h)).min((8.0e6 / (w * h)).sqrt()).min(2.0);
        if scale < 0.05 {
            return Err("too large to render as an image; ask for `svg`".into());
        }
        rhizo_render::svg_to_png(&svg, &fonts, scale).map(Rendered::Png).map_err(|e| e.to_string())
    }

    /// Replaces the whole document. Refused if the text has errors; nothing
    /// is written then.
    pub fn write_document(&self, source: &str, file: Option<&str>) -> Result<Value, String> {
        self.save(&self.target(file)?, source)
    }

    /// The file a write goes to: the served file, or `file` in a project
    /// folder (relative, `.rhizo`, inside the folder).
    fn target(&self, file: Option<&str>) -> Result<PathBuf, String> {
        if !self.path.is_dir() {
            return match file {
                Some(f) if f != self.name() => Err(format!("this server edits only {}", self.name())),
                _ => Ok(self.path.clone()),
            };
        }
        let f = file.ok_or("this is a project folder; say which `file` to write (e.g. `api.rhizo`)")?;
        if !rhizo_compile::safe_path(f) || !(f.ends_with(".rhizo") || f.ends_with(".kdl")) {
            return Err(format!("`{f}`: give a .rhizo path inside the project, like `flows/checkout.rhizo`"));
        }
        inside(&self.path, f)
    }

    /// Replaces `old` (which must occur exactly once) with `new`, keeping the
    /// rest of the file byte for byte. Refused if the result has errors.
    pub fn edit_document(&self, old: &str, new: &str, file: Option<&str>) -> Result<Value, String> {
        let target = self.target(file)?;
        let src = std::fs::read_to_string(&target).map_err(|e| format!("cannot read {}: {e}", target.display()))?;
        match src.matches(old).count() {
            0 => return Err("`old` does not occur in the document; read it again (it may have changed)".into()),
            1 => {}
            n => return Err(format!("`old` occurs {n} times; include more surrounding text so it is unique")),
        }
        if old.is_empty() {
            return Err("`old` is empty".into());
        }
        self.save(&target, &src.replacen(old, new, 1))
    }

    /// Adopts imported infrastructure nodes (or, with `all`, every node that
    /// can be): rhizo generates their code from then on. Runs OpenTofu for
    /// L1-adopt and writes nothing unless it passes. `var_files` are
    /// `.tfvars` files (relative to the served folder) both plans read.
    /// Providers beyond rhizo's allow-list come only from the person, at
    /// server start ([`Workspace::allow_providers`]).
    pub fn adopt_nodes(
        &self,
        nodes: &[String],
        all: bool,
        var_files: &[String],
        file: Option<&str>,
    ) -> Result<Value, String> {
        let target = self.target(file)?;
        let src = std::fs::read_to_string(&target).map_err(|e| format!("cannot read {}: {e}", target.display()))?;
        let base = if self.path.is_dir() {
            self.path.clone()
        } else {
            self.path.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        // The agent names them: only .tfvars files inside the served folder,
        // or OpenTofu's errors would quote any file back to it.
        let inputs = var_files
            .iter()
            .map(|f| match rhizo_compile::safe_path(f) && (f.ends_with(".tfvars") || f.ends_with(".tfvars.json")) {
                true => inside(&base, f),
                false => Err(format!("`{f}`: give a .tfvars file inside the served folder, like `prod.tfvars`")),
            })
            .collect::<Result<Vec<PathBuf>, String>>()?;
        let opts = rhizo_iac::Options { allow_providers: self.allow_providers.clone(), ..Default::default() };
        let adopted = rhizo_iac::adopt(&src, nodes, all, &opts, &inputs).map_err(|e| e.to_string())?;
        let mut out = self.save(&target, &adopted.kdl)?;
        merge(&mut out, json!({ "report": adopted.report }));
        Ok(out)
    }

    fn save(&self, path: &Path, text: &str) -> Result<Value, String> {
        if Format::from_path(path) != Some(Format::Rhizo) {
            return Err("only .rhizo/.kdl files can be written".into());
        }
        let report = self.report(text);
        if report["ok"] != true {
            return Err(format!(
                "not written: the text has errors\n{}",
                serde_json::to_string_pretty(&report).unwrap_or_default()
            ));
        }
        // Write next to the target, then rename: readers (the viewer) never see half a file.
        // shortcut: no lock against an edit made in the viewer at the same instant.
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        rhizo_project::write_atomic(path, text.as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(
            json!({ "written": path.display().to_string(), "bytes": text.len(), "warnings": report["warnings"], "diagnostics": report["diagnostics"] }),
        )
    }

    /// The kit catalog, one kit's types, or one type with its props schema.
    pub fn types(&self, name: Option<&str>) -> Result<Value, String> {
        if let Some(def) = name.and_then(|n| self.kits.find_type(n)) {
            let ports: Vec<Value> = def.ports.iter().map(|p| json!({ "port": p.port, "default": p.default })).collect();
            return Ok(json!({
                "type": def.name,
                "label": def.label,
                "description": def.description,
                "frame": def.frame,
                "parents": def.parents,
                "ports": ports,
                "props_schema": def.schema,
            }));
        }
        let kits: Vec<Value> = self
            .kits
            .iter()
            .filter(|k| name.is_none_or(|n| n == k.name))
            .map(|k| {
                let types: Vec<Value> = k
                    .types
                    .values()
                    .map(|t| json!({ "type": t.name, "label": t.label, "frame": t.frame, "props": t.prop_names() }))
                    .collect();
                json!({ "kit": k.name, "version": k.version.to_string(), "description": k.description, "types": types })
            })
            .collect();
        if kits.is_empty() {
            return Err(format!("no kit or type `{}`", name.unwrap_or_default()));
        }
        Ok(json!({ "kits": kits }))
    }

    /// Compiles in memory. Without `path`: the file list. With it: that
    /// file's contents. Nothing is written.
    pub fn compile_preview(&self, path: Option<&str>) -> Result<Value, String> {
        let bundle = self.load()?.compile(&self.kits).map_err(|findings| {
            let msgs: Vec<String> = findings.iter().map(|f| f.diagnostic.message.clone()).collect();
            format!("cannot compile: {}", msgs.join("; "))
        })?;
        match path {
            Some(p) => {
                let f = bundle.files.get(p).ok_or_else(|| {
                    format!("no file `{p}`; files: {}", bundle.files.keys().cloned().collect::<Vec<_>>().join(", "))
                })?;
                Ok(json!({ "path": p, "nodes": f.nodes, "contents": f.contents }))
            }
            None => {
                let files: Vec<Value> = bundle
                    .files
                    .iter()
                    .map(|(p, f)| json!({ "path": p, "bytes": f.contents.len(), "nodes": f.nodes }))
                    .collect();
                Ok(json!({ "files": files }))
            }
        }
    }

    /// Stores a proposed model change for a person to review. The document
    /// itself is never changed by this server.
    /// Records a proposed change for a person to apply (`rhizo proposals`).
    /// The document itself is never changed here.
    pub fn propose_change(&self, file: Option<&str>, old: &str, new: &str, rationale: &str) -> Result<Value, String> {
        let (root, rel) = if self.path.is_dir() {
            let rel = file.ok_or("this is a project folder; say which `file` the change is for")?;
            (self.path.clone(), rel.to_string())
        } else {
            let root = self.path.parent().map(Path::to_path_buf).unwrap_or_default();
            (root, file.map(str::to_string).unwrap_or_else(|| self.name()))
        };
        let p = rhizo_project::proposals::propose(&root, &rel, old, new, rationale, &self.kits)?;
        Ok(json!({
            "proposal": p.id,
            "file": p.file,
            "stored": root.join(format!(".rhizo/proposals/{:04}.json", p.id)).display().to_string(),
            "applied": false,
            "how": format!("a person applies it with `rhizo proposals <project> --apply {}`", p.id),
        }))
    }
}

fn node<'a>(doc: &'a Document, id: &str) -> Result<&'a Node, String> {
    doc.nodes.get(&NodeId(id.to_string())).ok_or_else(|| {
        let by_label = doc.nodes.values().find(|n| n.label.eq_ignore_ascii_case(id));
        match by_label {
            Some(n) => format!("no node `{id}`; did you mean `{}` (labelled \"{}\")?", n.id, n.label),
            None => format!("no node `{id}`; `list_nodes` finds ids"),
        }
    })
}

/// `data` matches `data.*`; `data.entity` matches exactly.
fn type_matches(n: &Node, want: &str) -> bool {
    let Some(ty) = &n.ty else { return false };
    let ty = ty.as_str();
    ty == want || ty.strip_prefix(want).is_some_and(|rest| rest.starts_with('.'))
}

fn summary(n: &Node) -> Value {
    json!({
        "id": n.id.as_str(),
        "label": n.label,
        "type": n.ty.as_ref().map(|t| t.as_str()),
        "frame": n.frame.as_str(),
        "parent": n.parent.as_ref().map(NodeId::as_str),
    })
}

fn edge(e: &Edge) -> Value {
    let mut v = json!({ "from": e.from.as_str(), "to": e.to.as_str() });
    let extra = json!({
        "id": e.id.as_ref().map(|i| i.as_str()),
        "label": e.label,
        "kind": e.kind.map(|k| k.name()),
        "type": e.ty.as_ref().map(|t| t.as_str()),
        "from_port": e.from_port.as_ref().map(|p| p.as_str()),
        "to_port": e.to_port.as_ref().map(|p| p.as_str()),
        "props": if e.props.is_empty() { Value::Null } else { json!(e.props) },
    });
    merge(&mut v, extra);
    v
}

/// Adds the non-null fields of `extra` to `v`.
fn merge(v: &mut Value, extra: Value) {
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e.into_iter().filter(|(_, x)| !x.is_null()));
    }
}

fn describe(src: &str, d: &Diagnostic) -> String {
    match d.line_col(src) {
        Some((l, c)) => format!("{l}:{c}: {}", d.message),
        None => d.message.clone(),
    }
}

fn diagnostic(src: &str, d: &Diagnostic) -> Value {
    let mut v = json!({
        "severity": if d.severity == Severity::Error { "error" } else { "warning" },
        "message": d.message,
    });
    let (line, column) = d.line_col(src).map_or((None, None), |(l, c)| (Some(l), Some(c)));
    merge(&mut v, json!({ "line": line, "column": column, "help": d.help }));
    v
}

/// `rel` (already a [`rhizo_compile::safe_path`]) under `root`, if no part
/// of the way is a symlink: one could lead outside the folder.
fn inside(root: &Path, rel: &str) -> Result<PathBuf, String> {
    let mut p = root.to_path_buf();
    for seg in rel.split('/') {
        p.push(seg);
        if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("`{rel}`: {} is a symlink; not going through it", p.display()));
        }
    }
    Ok(p)
}
