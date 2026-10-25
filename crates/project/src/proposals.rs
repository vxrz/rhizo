//! Proposals: changes an agent suggests and a person applies.
//!
//! A proposal is an exact text change to one file of a project (replace
//! `old`, which must occur exactly once, with `new`; or create a file when
//! `old` is empty), with the reason for it. It is checked when proposed and
//! again when applied, so applying never writes a broken diagram. Stored
//! in `<root>/.rhizo/proposals/NNNN.json`, with a Markdown copy for people.

use std::path::{Path, PathBuf};

use rhizo_interop::Format;
use rhizo_kit::Kits;
use serde::{Deserialize, Serialize};

use crate::Project;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Status {
    #[default]
    Open,
    Applied,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Proposal {
    pub id: u32,
    /// The file it changes, relative to the project root.
    pub file: String,
    pub rationale: String,
    /// Exact text to replace (occurs once); empty: create `file` with `new`.
    pub old: String,
    pub new: String,
    #[serde(default)]
    pub status: Status,
}

fn dir(root: &Path) -> PathBuf {
    root.join(".rhizo/proposals")
}

fn target(root: &Path, file: &str) -> Result<PathBuf, String> {
    // `:` would let `C:/x` (a drive on Windows) lead out of the project.
    let ok = !file.is_empty()
        && !file.starts_with('/')
        && file.split('/').all(|s| !s.is_empty() && s != "." && s != ".." && !s.contains(['\\', ':']))
        && (file.ends_with(".rhizo") || file.ends_with(".kdl"));
    if !ok {
        return Err(format!("`{file}`: give a .rhizo path inside the project, like `flows/checkout.rhizo`"));
    }
    if linked(root, file) {
        return Err(format!("`{file}` is reached through a symbolic link; refusing to read or write it"));
    }
    Ok(root.join(file))
}

/// True when a folder or the file on the way from `root` to `rel` is a
/// symbolic link: a cloned repository may plant one that points anywhere.
fn linked(root: &Path, rel: &str) -> bool {
    let mut p = root.to_path_buf();
    rel.split('/').any(|seg| {
        p.push(seg);
        std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink())
    })
}

/// The text `p` would leave in its file, or why it cannot apply now.
fn result_of(root: &Path, p: &Proposal) -> Result<String, String> {
    let path = target(root, &p.file)?;
    if p.old.is_empty() {
        if path.exists() {
            return Err(format!("{} exists already; a new file needs a new name", p.file));
        }
        return Ok(p.new.clone());
    }
    let src = crate::read_source(&path).map_err(|e| format!("cannot read {}: {e}", p.file))?;
    match src.matches(&p.old).count() {
        1 => Ok(src.replacen(&p.old, &p.new, 1)),
        0 => Err(format!("the text to replace is no longer in {} (the file changed since)", p.file)),
        n => Err(format!("the text to replace occurs {n} times in {}", p.file)),
    }
}

/// Errors in `text` as the file `rel` (syntax, structure, kit rules).
fn problems(rel: &str, text: &str, kits: &Kits) -> Vec<String> {
    let p = Project::open_text(rel, text, Format::Rhizo);
    p.check(kits).into_iter().flatten().filter(|d| d.is_error()).map(|d| d.message).collect()
}

/// Records a proposal, after checking it applies cleanly now.
pub fn propose(
    root: &Path,
    file: &str,
    old: &str,
    new: &str,
    rationale: &str,
    kits: &Kits,
) -> Result<Proposal, String> {
    if rationale.trim().is_empty() {
        return Err("a proposal needs a rationale: why the change is needed".into());
    }
    let mut p = Proposal {
        id: 0,
        file: file.to_string(),
        rationale: rationale.trim().to_string(),
        old: old.to_string(),
        new: new.to_string(),
        status: Status::Open,
    };
    let text = result_of(root, &p)?;
    let errs = problems(file, &text, kits);
    if !errs.is_empty() {
        return Err(format!("the change would leave errors: {}", errs.join("; ")));
    }
    let d = dir(root);
    if linked(root, ".rhizo/proposals") {
        return Err(format!("{} is reached through a symbolic link; refusing to write there", d.display()));
    }
    std::fs::create_dir_all(&d).map_err(|e| format!("cannot create {}: {e}", d.display()))?;
    // create_new: two proposals at once never take the same number.
    for _ in 0..64 {
        let last = list(root).iter().map(|q| q.id).max().unwrap_or(0);
        p.id = last.checked_add(1).ok_or("could not number the proposal")?;
        let json = d.join(format!("{:04}.json", p.id));
        let created = std::fs::OpenOptions::new().write(true).create_new(true).open(&json);
        match created {
            Ok(mut f) => {
                use std::io::Write;
                let body = serde_json::to_string_pretty(&p).expect("proposals serialize") + "\n";
                f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
                // create_new: never through a planted link, never over a file.
                let md =
                    std::fs::OpenOptions::new().write(true).create_new(true).open(d.join(format!("{:04}.md", p.id)));
                let _ = md.and_then(|mut f| f.write_all(markdown(&p).as_bytes()));
                return Ok(p);
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("cannot write {}: {e}", json.display())),
        }
    }
    Err("could not number the proposal".into())
}

/// Every proposal, by number.
pub fn list(root: &Path) -> Vec<Proposal> {
    let Ok(entries) = std::fs::read_dir(dir(root)) else { return Vec::new() };
    let mut out: Vec<Proposal> = entries
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .filter_map(|e| serde_json::from_str(&crate::read_source(&e.path()).ok()?).ok())
        .collect();
    out.sort_by_key(|p| p.id);
    out
}

fn save(root: &Path, p: &Proposal) -> Result<(), String> {
    let path = dir(root).join(format!("{:04}.json", p.id));
    if linked(root, &format!(".rhizo/proposals/{:04}.json", p.id)) {
        return Err(format!("{} is reached through a symbolic link; refusing to write it", path.display()));
    }
    let body = serde_json::to_string_pretty(p).expect("proposals serialize") + "\n";
    crate::write_atomic(&path, body.as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))
}

fn find(root: &Path, id: u32) -> Result<Proposal, String> {
    list(root).into_iter().find(|p| p.id == id).ok_or_else(|| format!("no proposal {id}"))
}

/// Applies an open proposal: checks it still fits the file and leaves no
/// errors, writes the file, marks the proposal applied. Returns the file.
pub fn apply(root: &Path, id: u32, kits: &Kits) -> Result<PathBuf, String> {
    let mut p = find(root, id)?;
    if p.status != Status::Open {
        return Err(format!("proposal {id} is {:?}, not open", p.status).to_lowercase());
    }
    let text = result_of(root, &p)?;
    let errs = problems(&p.file, &text, kits);
    if !errs.is_empty() {
        return Err(format!("not applied: it would leave errors: {}", errs.join("; ")));
    }
    let path = target(root, &p.file)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    crate::write_atomic(&path, text.as_bytes()).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    p.status = Status::Applied;
    save(root, &p)?;
    Ok(path)
}

pub fn reject(root: &Path, id: u32) -> Result<(), String> {
    let mut p = find(root, id)?;
    p.status = Status::Rejected;
    save(root, &p)
}

/// The proposal for a person to read: why, which file, the old and new text.
pub fn markdown(p: &Proposal) -> String {
    let fence = |t: &str| {
        // Longer than any run of backticks in the text, so it cannot break out.
        let longest = t.split(|c| c != '`').map(str::len).max().unwrap_or(0);
        "`".repeat(longest.max(2) + 1)
    };
    let mut s = format!("# Proposal {:04}: {}\n\n{}\n\n", p.id, p.file, p.rationale);
    if p.old.is_empty() {
        let f = fence(&p.new);
        s += &format!("Creates `{}`:\n\n{f}kdl\n{}\n{f}\n", p.file, p.new.trim_end());
    } else {
        let (fo, fnw) = (fence(&p.old), fence(&p.new));
        s += &format!(
            "Replaces\n\n{fo}kdl\n{}\n{fo}\n\nwith\n\n{fnw}kdl\n{}\n{fnw}\n",
            p.old.trim_end(),
            p.new.trim_end()
        );
    }
    s += &format!("\nApply: `rhizo proposals <project> --apply {}` · reject: `--reject {}`\n", p.id, p.id);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rhizo-proposals-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("a.rhizo"), "rhizo version=1\n- \"Plan\" {\n    - \"Build\"\n}\n").unwrap();
        d
    }

    #[test]
    fn propose_list_apply() {
        let (r, kits) = (root("apply"), Kits::builtin());
        let p =
            propose(&r, "a.rhizo", "    - \"Build\"\n", "    - \"Build\"\n    - \"Ship\"\n", "Ship it", &kits).unwrap();
        assert_eq!((p.id, p.status), (1, Status::Open));
        assert!(std::fs::read_to_string(r.join(".rhizo/proposals/0001.md")).unwrap().contains("Ship it"));
        assert_eq!(std::fs::read_to_string(r.join("a.rhizo")).unwrap().matches("Ship").count(), 0, "not applied yet");
        apply(&r, 1, &kits).unwrap();
        assert!(std::fs::read_to_string(r.join("a.rhizo")).unwrap().contains("- \"Ship\""));
        assert_eq!(list(&r)[0].status, Status::Applied);
        assert!(apply(&r, 1, &kits).is_err(), "only once");
        std::fs::remove_dir_all(r).unwrap();
    }

    #[test]
    fn refuses_what_would_break_or_no_longer_fits() {
        let (r, kits) = (root("refuse"), Kits::builtin());
        assert!(propose(&r, "a.rhizo", "- \"Plan\"", "- ", "x", &kits).unwrap_err().contains("errors"));
        assert!(propose(&r, "../x.rhizo", "", "rhizo version=1\n", "x", &kits).is_err());
        assert!(propose(&r, "a.rhizo", "nowhere", "x", "x", &kits).is_err());
        assert!(propose(&r, "a.rhizo", "\"Build\"", "\"Make\"", "", &kits).is_err(), "needs a reason");
        let p = propose(&r, "a.rhizo", "\"Build\"", "\"Make\"", "Clearer", &kits).unwrap();
        std::fs::write(r.join("a.rhizo"), "rhizo version=1\n- \"Other\"\n").unwrap();
        assert!(apply(&r, p.id, &kits).unwrap_err().contains("no longer"));
        reject(&r, p.id).unwrap();
        assert_eq!(list(&r)[0].status, Status::Rejected);
        // New files.
        let n = propose(&r, "flows/new.rhizo", "", "rhizo version=1\n- \"New\"\n", "A flow", &kits).unwrap();
        apply(&r, n.id, &kits).unwrap();
        assert!(r.join("flows/new.rhizo").exists());
        std::fs::remove_dir_all(r).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn planted_links_are_never_written_through() {
        let (r, kits) = (root("links"), Kits::builtin());
        let victim = r.with_extension("victim");
        std::fs::write(&victim, "keep").unwrap();
        // A cloned repository can carry a link where the proposal's notes go.
        std::fs::create_dir_all(r.join(".rhizo/proposals")).unwrap();
        std::os::unix::fs::symlink(&victim, r.join(".rhizo/proposals/0001.md")).unwrap();
        propose(&r, "a.rhizo", "\"Build\"", "\"Make\"", "Clearer", &kits).unwrap();
        assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
        // ... or a folder of the project that leads elsewhere.
        std::os::unix::fs::symlink(std::env::temp_dir(), r.join("flows")).unwrap();
        assert!(propose(&r, "flows/x.rhizo", "", "rhizo version=1\n", "x", &kits).unwrap_err().contains("link"));
        assert!(propose(&r, "C:/x.rhizo", "", "rhizo version=1\n", "x", &kits).is_err());
        let _ = std::fs::remove_dir_all(r.join(".rhizo"));
        std::os::unix::fs::symlink(std::env::temp_dir(), r.join(".rhizo")).unwrap();
        assert!(propose(&r, "a.rhizo", "\"Build\"", "\"Make\"", "Clearer", &kits).unwrap_err().contains("link"));
        std::fs::remove_dir_all(r).unwrap();
        std::fs::remove_file(victim).unwrap();
    }

    #[test]
    fn markdown_fences_cannot_be_broken() {
        let p = Proposal {
            id: 3,
            file: "a.rhizo".into(),
            rationale: "r".into(),
            old: "x".into(),
            new: "```\nevil".into(),
            status: Status::Open,
        };
        assert!(markdown(&p).contains("````kdl\n```\nevil\n````"));
    }
}
