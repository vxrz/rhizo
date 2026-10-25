//! Files: what the viewer can open, import and export, the recent files
//! list, and handing a file to the system. No dialogs here, so it is testable.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use rhizo_interop::Format;
use rhizo_lang::Syntax;
use rhizo_render::SvgOptions;

use crate::Built;

/// How many recent files are kept.
pub const MAX_RECENT: usize = 10;

/// Export choices, in the order the save dialog lists them.
pub const EXPORTS: [Format; 14] = [
    Format::Svg,
    Format::Png,
    Format::Pdf,
    Format::Mermaid,
    Format::D2,
    Format::Dot,
    Format::DrawIo,
    Format::JsonCanvas,
    Format::Opml,
    Format::FreeMind,
    Format::GraphMl,
    Format::Markdown,
    Format::Json,
    Format::Yaml,
];

/// A name for the save dialog's format list.
pub fn label(f: Format) -> &'static str {
    match f {
        Format::Rhizo => "rhizo",
        Format::Svg => "SVG image",
        Format::Png => "PNG image",
        Format::Pdf => "PDF document",
        Format::Mermaid => "Mermaid",
        Format::D2 => "D2",
        Format::Dot => "Graphviz DOT",
        Format::DrawIo => "draw.io",
        Format::JsonCanvas => "JSON Canvas",
        Format::Opml => "OPML outline",
        Format::FreeMind => "FreeMind",
        Format::GraphMl => "GraphML",
        Format::Markdown => "Markdown outline",
        Format::Json => "rhizo JSON",
        Format::Yaml => "rhizo YAML",
    }
}

/// The syntax a rhizo file is written in, by extension.
pub fn syntax_of(path: &Path) -> Syntax {
    Format::from_path(path).and_then(Format::syntax).unwrap_or(Syntax::Kdl)
}

/// What opening `path` takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opening {
    /// A rhizo document (KDL, JSON or YAML): view it as it is.
    Direct,
    /// A foreign format: import it, then save it as `.rhizo`.
    Import(Format),
    Unsupported,
}

pub fn opening(path: &Path) -> Opening {
    match Format::from_path(path) {
        Some(Format::Rhizo | Format::Json | Format::Yaml) => Opening::Direct,
        Some(f) if f.can_import() => Opening::Import(f),
        _ => Opening::Unsupported,
    }
}

/// Every extension the open dialog accepts.
pub fn openable_extensions() -> Vec<&'static str> {
    Format::ALL.into_iter().filter(|f| f.can_import()).flat_map(|f| f.extensions().iter().copied()).collect()
}

/// Where an imported file is suggested to go: beside it, as `.rhizo`.
pub fn import_target(src: &Path) -> PathBuf {
    src.with_extension("rhizo")
}

/// Reads a foreign file and returns it as rhizo KDL.
pub fn import_file(src: &Path, format: Format) -> Result<String, String> {
    let text = std::fs::read_to_string(src).map_err(|e| format!("cannot read {}: {e}", src.display()))?;
    match rhizo_interop::import(&text, format) {
        Ok((doc, _warnings)) => Ok(rhizo_lang::to_kdl(&doc)),
        Err(errs) => {
            let first = errs.iter().find(|d| d.is_error()).or(errs.first());
            Err(format!("cannot import {}: {}", name(src), first.map_or("unknown error", |d| d.message.as_str())))
        }
    }
}

/// The export format for a chosen file name, from its extension.
pub fn export_format(path: &Path) -> Option<Format> {
    Format::from_path(path).filter(|f| EXPORTS.contains(f))
}

/// What `built` shows, as `format`, mirroring `rhizo render`/`rhizo convert`:
/// drawings and foreign formats as drawn (references resolved, kit looks),
/// JSON and YAML as written.
pub fn export(built: &Built, format: Format, theme: Option<&str>) -> Result<Vec<u8>, String> {
    let fonts = built.fonts.as_ref();
    let written;
    let doc = if format.syntax().is_some() {
        let loaded = rhizo_lang::load(&built.source, built.syntax).into_result();
        written = loaded.map_err(|_| "the file has errors".to_string())?.0;
        &written
    } else {
        &built.document
    };
    let theme = rhizo_render::theme_for(doc, theme);
    if format.is_image() {
        let diagram = rhizo_render::layout(doc, &theme, fonts);
        let svg = rhizo_render::to_svg(&diagram, &SvgOptions::default());
        return match format {
            Format::Svg => Ok(svg.into_bytes()),
            Format::Png => rhizo_render::svg_to_png(&svg, fonts, 2.0).map_err(|e| e.to_string()),
            _ => rhizo_render::svg_to_pdf(&svg, fonts, doc.title.as_deref()).map_err(|e| e.to_string()),
        };
    }
    rhizo_interop::export(doc, format, &theme, fonts).map(String::into_bytes)
}

/// The file at `from` (in `from`'s syntax) rewritten for `to`: as is when
/// both use the same syntax, converted otherwise.
pub fn save_as_text(text: &str, from: &Path, to: &Path) -> Result<String, String> {
    let (a, b) = (syntax_of(from), syntax_of(to));
    if a == b {
        return Ok(text.to_string());
    }
    let (doc, _) = rhizo_lang::load(text, a)
        .into_result()
        .map_err(|_| "fix the problems in the file before saving it in another syntax".to_string())?;
    Ok(match b {
        Syntax::Kdl => rhizo_lang::to_kdl(&doc),
        Syntax::Json => rhizo_lang::to_json(&doc),
        Syntax::Yaml => rhizo_lang::to_yaml(&doc),
    })
}

/// Writes through a temporary file, so a watcher never sees half a file.
pub fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    rhizo_project::write_atomic(path, contents)
}

/// Creates `path` with `contents`, failing if it exists (an agent may have
/// just written it).
pub fn write_new(path: &Path, contents: &str) -> std::io::Result<()> {
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(contents.as_bytes())
}

pub fn name(path: &Path) -> String {
    path.file_name().map_or_else(|| path.display().to_string(), |n| n.to_string_lossy().to_string())
}

fn home() -> Option<PathBuf> {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).filter(|h| !h.is_empty()).map(PathBuf::from)
}

/// `path` for people: the home directory as `~`.
pub fn display_path(path: &Path) -> String {
    match home().and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
        Some(rest) if rest.as_os_str().is_empty() => "~".into(),
        Some(rest) => format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display()),
        None => path.display().to_string(),
    }
}

/// The platform's per-user config directory.
pub fn config_dir() -> Option<PathBuf> {
    let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        home().map(|h| h.join("Library/Application Support"))
    } else if cfg!(windows) {
        var("APPDATA")
    } else {
        var("XDG_CONFIG_HOME").or_else(|| home().map(|h| h.join(".config")))
    }
}

/// Where the recent files list lives.
pub fn recent_file() -> Option<PathBuf> {
    config_dir().map(|d| d.join("rhizo").join("recent.json"))
}

/// Recently opened files and folders, most recent first.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Recent {
    pub files: Vec<PathBuf>,
}

impl Recent {
    /// The list at `at` (files and folders), without those that are gone.
    /// Missing or corrupt lists read as empty.
    pub fn load(at: &Path) -> Recent {
        let files: Vec<String> =
            std::fs::read_to_string(at).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mut r = Recent::default();
        for f in files.into_iter().rev() {
            r.add(PathBuf::from(f));
        }
        r.files.retain(|p| p.exists());
        r
    }

    pub fn save(&self, at: &Path) -> std::io::Result<()> {
        // shortcut: paths that are not UTF-8 are not remembered; store bytes if that matters.
        let files: Vec<&str> = self.files.iter().filter_map(|p| p.to_str()).collect();
        if let Some(dir) = at.parent() {
            std::fs::create_dir_all(dir)?;
        }
        write_atomic(at, serde_json::to_string_pretty(&files).expect("strings serialize").as_bytes())
    }

    /// Puts `path` first, once, keeping at most [`MAX_RECENT`].
    pub fn add(&mut self, path: PathBuf) {
        self.files.retain(|p| *p != path);
        self.files.insert(0, path);
        self.files.truncate(MAX_RECENT);
    }
}

/// Adds `path` to the stored recent files.
pub fn remember(path: &Path) {
    let Some(at) = recent_file() else { return };
    let mut recent = Recent::load(&at);
    recent.add(path.to_path_buf());
    if let Err(e) = recent.save(&at) {
        eprintln!("cannot save recent files to {}: {e}", at.display());
    }
}

/// The stored recent files.
pub fn recent() -> Vec<PathBuf> {
    recent_file().map(|at| Recent::load(&at).files).unwrap_or_default()
}

/// Writes `text` to a new file `path`, replacing one this user left there.
/// Never through a link: in a shared temporary folder (`/tmp`) another user
/// could plant one at a name rhizo always uses.
pub fn write_fresh(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    let _ = std::fs::remove_file(path);
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(text.as_bytes())
}

/// Opens `path` in the system's text editor (macOS) or default app.
pub fn open_in_editor(path: &Path) -> std::io::Result<()> {
    let mut cmd = if cfg!(target_os = "macos") {
        let mut c = Command::new("open");
        c.arg("-t");
        c
    } else if cfg!(windows) {
        Command::new("explorer")
    } else {
        Command::new("xdg-open")
    };
    spawn(cmd.arg(path))
}

/// Opens a web address in the browser.
pub fn open_url(url: &str) -> std::io::Result<()> {
    let program = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    spawn(Command::new(program).arg(url))
}

/// Shows `path` in the file manager (selected, where the platform can).
pub fn reveal(path: &Path) -> std::io::Result<()> {
    let mut cmd;
    if cfg!(target_os = "macos") {
        cmd = Command::new("open");
        cmd.arg("-R").arg(path);
    } else if cfg!(windows) {
        // shortcut: explorer wants `/select,` and the path in one argument;
        // paths with commas may not select. Use raw_arg if that shows up.
        cmd = Command::new("explorer");
        cmd.arg(format!("/select,{}", path.display()));
    } else {
        cmd = Command::new("xdg-open");
        cmd.arg(path.parent().unwrap_or(path));
    }
    spawn(&mut cmd)
}

/// Starts `cmd` and reaps it in the background, so no zombie is left.
fn spawn(cmd: &mut Command) -> std::io::Result<()> {
    let mut child = cmd.spawn()?;
    std::thread::spawn(move || child.wait());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn fresh_files_replace_links_instead_of_writing_through_them() {
        let dir = std::env::temp_dir().join(format!("rhizo-fresh-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (victim, at) = (dir.join("victim"), dir.join("rhizo-guide.md"));
        std::fs::write(&victim, "keep").unwrap();
        std::os::unix::fs::symlink(&victim, &at).unwrap();
        write_fresh(&at, "guide").unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        assert_eq!(std::fs::read_to_string(&at).unwrap(), "guide");
        write_fresh(&at, "again").unwrap();
        assert_eq!(std::fs::read_to_string(&at).unwrap(), "again");
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rhizo-files-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn recent_order_dedupe_cap() {
        let mut r = Recent::default();
        for i in 0..12 {
            r.add(PathBuf::from(format!("/f{i}")));
        }
        assert_eq!(r.files.len(), MAX_RECENT);
        assert_eq!(r.files[0], PathBuf::from("/f11"));
        r.add(PathBuf::from("/f5"));
        assert_eq!(r.files[0], PathBuf::from("/f5"));
        assert_eq!(r.files.iter().filter(|p| p.as_path() == Path::new("/f5")).count(), 1);
        assert_eq!(r.files.len(), MAX_RECENT);
    }

    #[test]
    fn recent_round_trip_prunes_and_survives_corruption() {
        let dir = temp("recent");
        let at = dir.join("nested/recent.json");
        assert_eq!(Recent::load(&at), Recent::default(), "missing reads as empty");
        let (a, b) = (dir.join("a.rhizo"), dir.join("b.rhizo"));
        std::fs::write(&a, "").unwrap();
        std::fs::write(&b, "").unwrap();
        let mut r = Recent::default();
        r.add(a.clone());
        r.add(dir.join("gone.rhizo"));
        r.add(b.clone());
        r.save(&at).unwrap();
        assert_eq!(Recent::load(&at).files, vec![b, a], "order kept, missing file pruned");
        std::fs::write(&at, "{ not json").unwrap();
        assert_eq!(Recent::load(&at), Recent::default(), "corrupt reads as empty");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn what_opening_takes() {
        assert_eq!(opening(Path::new("a.rhizo")), Opening::Direct);
        assert_eq!(opening(Path::new("a.KDL")), Opening::Direct);
        assert_eq!(opening(Path::new("a.yml")), Opening::Direct);
        assert_eq!(opening(Path::new("a.json")), Opening::Direct);
        assert_eq!(opening(Path::new("a.mmd")), Opening::Import(Format::Mermaid));
        assert_eq!(opening(Path::new("notes.md")), Opening::Import(Format::Markdown));
        assert_eq!(opening(Path::new("a.canvas")), Opening::Import(Format::JsonCanvas));
        assert_eq!(opening(Path::new("a.svg")), Opening::Unsupported);
        assert_eq!(opening(Path::new("README")), Opening::Unsupported);
        assert!(openable_extensions().contains(&"opml"));
        assert_eq!(import_target(Path::new("/x/notes.md")), PathBuf::from("/x/notes.rhizo"));
        assert_eq!(import_target(Path::new("/x/my.map.mm")), PathBuf::from("/x/my.map.rhizo"));
    }

    #[test]
    fn export_format_from_extension() {
        assert_eq!(export_format(Path::new("a.svg")), Some(Format::Svg));
        assert_eq!(export_format(Path::new("a.PNG")), Some(Format::Png));
        assert_eq!(export_format(Path::new("a.gv")), Some(Format::Dot));
        assert_eq!(export_format(Path::new("a.drawio")), Some(Format::DrawIo));
        assert_eq!(export_format(Path::new("a.yml")), Some(Format::Yaml));
        assert_eq!(export_format(Path::new("a.rhizo")), None, "that is Save As");
        assert_eq!(export_format(Path::new("a")), None);
    }

    #[test]
    fn imports_and_exports_every_format() {
        let dir = temp("convert");
        let md = dir.join("notes.md");
        std::fs::write(&md, "# Plan\n- Why\n- How\n").unwrap();
        let kdl = import_file(&md, Format::Markdown).unwrap();
        assert!(kdl.contains("Plan") && kdl.contains("How"), "{kdl}");
        assert!(import_file(&dir.join("missing.md"), Format::Markdown).is_err());

        let src = rhizo_lang::starters::starter("flow").unwrap();
        let built = crate::Engine::default().build("flow", src, Syntax::Kdl, Some("dark")).unwrap();
        for f in EXPORTS {
            let bytes = export(&built, f, Some("dark")).unwrap_or_else(|e| panic!("{f:?}: {e}"));
            assert!(!bytes.is_empty(), "{f:?}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn save_as_converts_between_syntaxes() {
        let src = rhizo_lang::starters::starter("mindmap").unwrap();
        let same = save_as_text(src, Path::new("a.rhizo"), Path::new("b.kdl")).unwrap();
        assert_eq!(same, src, "copied verbatim, comments and all");
        let json = save_as_text(src, Path::new("a.rhizo"), Path::new("b.json")).unwrap();
        assert!(rhizo_lang::load(&json, Syntax::Json).diagnostics.is_empty(), "{json}");
        assert!(save_as_text("node {", Path::new("a.rhizo"), Path::new("b.yaml")).is_err());
    }

    #[test]
    fn write_new_never_overwrites() {
        let dir = temp("new");
        let p = dir.join("a.rhizo");
        write_new(&p, "one").unwrap();
        assert!(write_new(&p, "two").is_err());
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one");
        write_atomic(&p, b"three").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "three");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn paths_for_people() {
        if let Some(h) = home() {
            assert_eq!(
                display_path(&h.join("a").join("b.rhizo")),
                format!("~{0}a{0}b.rhizo", std::path::MAIN_SEPARATOR)
            );
        }
        assert_eq!(name(Path::new("/x/y.rhizo")), "y.rhizo");
    }
}
