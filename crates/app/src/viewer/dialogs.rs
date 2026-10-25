//! Native file dialogs and what runs through them: opening, importing,
//! new files from templates, save as, export, handing a file to the system.

use std::path::{Path, PathBuf};

use rhizo_app::files::{self, Opening};
use rhizo_interop::Format;
use winit::keyboard::ModifiersState;

use super::App;

impl App {
    pub fn open_folder_dialog(&mut self) {
        if let Some(dir) = self.ask(|d| d.set_title("Open folder").pick_folder()) {
            self.open_folder(dir);
        }
    }

    /// Runs a native dialog. Synchronous: the window waits while it is open.
    pub fn ask(&mut self, f: impl FnOnce(rfd::FileDialog) -> Option<PathBuf>) -> Option<PathBuf> {
        let mut d = rfd::FileDialog::new();
        let dir =
            self.session.path.as_deref().or(self.session.recent.first().map(PathBuf::as_path)).and_then(Path::parent);
        if let Some(dir) = dir {
            d = d.set_directory(dir);
        }
        if let Some(w) = self.window() {
            d = d.set_parent(w.as_ref());
        }
        let chosen = f(d);
        // The dialog took the key-up of ⌘ and friends.
        self.modifiers = ModifiersState::empty();
        chosen
    }

    /// Asks where to save a rhizo file; `None` if cancelled or not a rhizo file name.
    pub fn ask_rhizo(&mut self, title: &str, dir: Option<&Path>, name: &str) -> Option<PathBuf> {
        let to = self.ask(|mut d| {
            if let Some(dir) = dir {
                d = d.set_directory(dir);
            }
            d.set_title(title)
                .set_file_name(name)
                .add_filter("rhizo", &["rhizo"])
                .add_filter("KDL", &["kdl"])
                .add_filter("rhizo JSON", &["json"])
                .add_filter("rhizo YAML", &["yaml", "yml"])
                .save_file()
        })?;
        if files::opening(&to) != Opening::Direct {
            self.fail(format!("{} is not a rhizo file name: end it in .rhizo", files::name(&to)));
            return None;
        }
        Some(to)
    }

    pub fn open_dialog(&mut self) {
        let exts = files::openable_extensions();
        if let Some(p) = self.ask(|d| d.set_title("Open").add_filter("Diagrams", &exts).pick_file()) {
            self.open(p);
        }
    }

    pub fn import(&mut self, src: &Path, format: Format) {
        let kdl = match files::import_file(src, format) {
            Ok(k) => k,
            Err(e) => return self.fail(e),
        };
        let target = files::import_target(src);
        let Some(to) = self.ask_rhizo("Save the imported diagram as", target.parent(), &files::name(&target)) else {
            return;
        };
        if std::fs::canonicalize(&to).is_ok_and(|t| t == src) {
            return self.fail("choose a new file: the original stays as it is".into());
        }
        let text = files::save_as_text(&kdl, Path::new("import.rhizo"), &to);
        match text.and_then(|t| files::write_atomic(&to, t.as_bytes()).map_err(|e| e.to_string())) {
            Ok(()) => {
                self.open(to.clone());
                self.toast(format!("Imported {} into {}", files::name(src), files::name(&to)));
            }
            Err(e) => self.fail(format!("cannot write {}: {e}", to.display())),
        }
    }

    pub fn new_file(&mut self, kind: &str) {
        let name = if kind == "learn" { "Learn rhizo.rhizo" } else { "Untitled.rhizo" };
        self.new_named(kind, name);
    }

    /// A new file from the starter of `kind`, saved where a dialog says (`name` proposed).
    pub fn new_named(&mut self, kind: &str, name: &str) {
        let Some(src) = rhizo_lang::starters::starter(kind) else { return };
        let Some(to) = self.ask_rhizo("New diagram", None, name) else { return };
        let text = files::save_as_text(src, Path::new("starter.rhizo"), &to);
        match text.and_then(|t| files::write_atomic(&to, t.as_bytes()).map_err(|e| e.to_string())) {
            Ok(()) => self.open(to),
            Err(e) => self.fail(format!("cannot write {}: {e}", to.display())),
        }
    }

    /// Creates the file the window is waiting for.
    pub fn create_missing(&mut self, kind: &str) {
        let (Some(path), Some(src)) = (self.session.path.clone(), rhizo_lang::starters::starter(kind)) else { return };
        let text = files::save_as_text(src, Path::new("starter.rhizo"), &path);
        match text.and_then(|t| files::write_new(&path, &t).map_err(|e| e.to_string())) {
            // The watcher picks it up.
            Ok(()) => files::remember(&path),
            Err(e) => self.fail(format!("cannot create {}: {e}", path.display())),
        }
    }

    /// Copies the file to a new place (converting between KDL, JSON and
    /// YAML if the extension asks) and continues there.
    pub fn save_as(&mut self) {
        let Some(from) = self.session.path.clone().filter(|p| p.is_file()) else { return };
        let text = match std::fs::read_to_string(&from) {
            Ok(t) => t,
            Err(e) => return self.fail(format!("cannot read {}: {e}", from.display())),
        };
        let Some(to) = self.ask_rhizo("Save as", from.parent(), &files::name(&from)) else { return };
        if std::fs::canonicalize(&to).is_ok_and(|t| t == from) {
            return;
        }
        match files::save_as_text(&text, &from, &to)
            .and_then(|t| files::write_atomic(&to, t.as_bytes()).map_err(|e| e.to_string()))
        {
            Ok(()) => {
                self.open(to.clone());
                self.toast(format!("Saved as {}", files::display_path(&to)));
            }
            Err(e) => self.fail(format!("cannot save {}: {e}", to.display())),
        }
    }

    /// Exports what the window shows, in the format the file name asks for.
    pub fn export(&mut self) {
        let Some(from) = self.session.path.clone().filter(|_| self.built.is_some()) else { return };
        let stem = from.file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| "diagram".into());
        let Some(to) = self.ask(|mut d| {
            d = d.set_title("Export").set_file_name(format!("{stem}.svg"));
            for f in files::EXPORTS {
                d = d.add_filter(files::label(f), f.extensions());
            }
            d.save_file()
        }) else {
            return;
        };
        let Some(format) = files::export_format(&to) else {
            return self
                .fail(format!("cannot tell the format of {}: end it in .svg, .png, .pdf, .mmd…", files::name(&to)));
        };
        if std::fs::canonicalize(&to).is_ok_and(|t| t == from) {
            return self.fail("that is the diagram itself: use Save as… to write it elsewhere".into());
        }
        // shortcut: exports run on the UI thread (PNG/PDF of a huge diagram can take a second); move to a thread if that bites.
        let Some(b) = &self.built else { return };
        match files::export(b, format, self.session.theme.as_deref())
            .and_then(|bytes| std::fs::write(&to, bytes).map_err(|e| format!("cannot write {}: {e}", to.display())))
        {
            Ok(()) => self.toast(format!("Exported {}", files::display_path(&to))),
            Err(e) => self.fail(e),
        }
    }

    pub fn hand_to_system(&mut self, f: fn(&Path) -> std::io::Result<()>) {
        let Some(path) = self.session.path.clone() else { return };
        if let Err(e) = f(&path) {
            self.fail(format!("cannot hand {} to the system: {e}", files::name(&path)));
        }
    }

    /// The open file (or its project) compiled, as `rhizo compile` does;
    /// `None` after saying why not.
    fn bundle(&mut self) -> Option<rhizo_compile::Bundle> {
        let path = self.session.folder.clone().or_else(|| self.session.path.clone())?;
        self.flush();
        let compiled = rhizo_project::Project::open(&path).and_then(|p| {
            p.compile(&self.inspector.kits)
                .map_err(|f| f.into_iter().map(|f| f.diagnostic.message).collect::<Vec<_>>().join("; "))
        });
        compiled.map_err(|e| self.fail(format!("cannot compile {}: {e}", files::name(&path)))).ok()
    }

    /// Diagram ▸ Compile specs…: the bundle `rhizo compile` writes, into a
    /// folder. Files it generated before and no longer does stay; `rhizo
    /// compile` removes them.
    pub fn compile_dialog(&mut self) {
        let Some(bundle) = self.bundle() else { return };
        let Some(out) = self.ask(|d| d.set_title("Compile specs into folder").pick_folder()) else { return };
        let mut written = 0;
        for (rel, file) in &bundle.files {
            let target = out.join(rel);
            // Like the CLI: a path of the bundle never leads out of the folder or through a link.
            let linked = rel.split('/').scan(out.clone(), |p, seg| {
                p.push(seg);
                Some(std::fs::symlink_metadata(&*p).is_ok_and(|m| m.file_type().is_symlink()))
            });
            if !rhizo_compile::safe_path(rel) || linked.into_iter().any(|l| l) {
                return self.fail(format!("refusing to write {}", target.display()));
            }
            if std::fs::read(&target).is_ok_and(|b| b == file.contents.as_bytes()) {
                continue;
            }
            let wrote = target
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&target, &file.contents));
            if let Err(e) = wrote {
                return self.fail(format!("cannot write {}: {e}", target.display()));
            }
            written += 1;
        }
        self.saw(|s| s.compiled = true);
        self.toast(format!(
            "Compiled into {}: {written} of {} files changed",
            files::display_path(&out),
            bundle.files.len()
        ));
    }

    /// Diagram ▸ Build brief: the brief an agent builds from (`rhizo
    /// brief`), opened in the text editor.
    pub fn brief(&mut self) {
        let Some(mut bundle) = self.bundle() else { return };
        let text = bundle.files.remove(rhizo_compile::AGENTS).map(|f| f.contents).unwrap_or_default();
        let name = self.session.path.as_deref().map_or("project".into(), files::name);
        self.show_text(&format!("{name}-brief.md"), &text);
    }

    /// Help ▸ Guide: the guide to writing rhizo (`rhizo guide`), opened in the text editor.
    pub fn guide(&mut self) {
        let text = self.inspector.kits.guide();
        self.show_text("rhizo-guide.md", &text);
    }

    /// Writes `text` to a file of the temporary folder and opens it in the text editor.
    fn show_text(&mut self, name: &str, text: &str) {
        let path = std::env::temp_dir().join(name);
        match files::write_fresh(&path, text).and_then(|()| files::open_in_editor(&path)) {
            Ok(()) => self.toast(format!("Opened {name}")),
            Err(e) => self.fail(format!("cannot open {}: {e}", path.display())),
        }
    }
}
