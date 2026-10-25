//! The repo's examples, built in: a fresh install has no checkout, yet
//! the viewer's gallery (File ▸ Open Sample…), `rhizo samples` and the MCP
//! `samples` tool all offer them. A sample is copied before it is opened;
//! the originals stay as they are.

use std::path::{Path, PathBuf};

/// One example: a file, or a project of several files that refer to each other.
#[derive(Debug)]
pub struct Sample {
    /// What `rhizo samples copy` takes.
    pub id: &'static str,
    pub title: &'static str,
    /// One line on what it shows.
    pub blurb: &'static str,
    pub group: &'static str,
    /// Paths relative to the copy, and contents. The first is the one to
    /// open (and the thumbnail's).
    pub files: &'static [(&'static str, &'static str)],
}

impl Sample {
    /// A folder of files, opened as a project.
    pub fn is_project(&self) -> bool {
        self.files.len() > 1
    }
}

/// The groups, in the order the gallery shows them.
pub const GROUPS: &[&str] =
    &["Mindmaps & notes", "Flows & sequences", "Data & APIs", "Infrastructure", "Projects", "Import & adopt"];

macro_rules! file {
    ($rel:literal, $path:literal) => {
        ($rel, include_str!(concat!("../../../examples/", $path)))
    };
}

pub const SAMPLES: &[Sample] = &[
    Sample {
        id: "product-strategy",
        title: "Product strategy",
        blurb: "Colours, notes and links",
        group: GROUPS[0],
        files: &[file!("product-strategy.rhizo", "product-strategy.rhizo")],
    },
    Sample {
        id: "mindmap",
        title: "Trip to Japan",
        blurb: "A mindmap with branch colours",
        group: GROUPS[0],
        files: &[file!("mindmap.rhizo", "playground/01-mindmap.rhizo")],
    },
    Sample {
        id: "canvas",
        title: "Sprint board",
        blurb: "Nodes stay where you put them",
        group: GROUPS[0],
        files: &[file!("canvas.rhizo", "playground/03-canvas.rhizo")],
    },
    Sample {
        id: "network",
        title: "Who influenced whom",
        blurb: "A network, laid out by forces",
        group: GROUPS[0],
        files: &[file!("network.rhizo", "playground/04-network.rhizo")],
    },
    Sample {
        id: "org-chart",
        title: "Org chart",
        blurb: "A tidy tree, top down",
        group: GROUPS[0],
        files: &[file!("org-chart.rhizo", "playground/05-org-chart.rhizo")],
    },
    Sample {
        id: "release-flow",
        title: "Release pipeline",
        blurb: "Decisions and loops",
        group: GROUPS[1],
        files: &[file!("release-flow.rhizo", "release-flow.rhizo")],
    },
    Sample {
        id: "flowchart",
        title: "Support ticket",
        blurb: "Edge labels and a back edge",
        group: GROUPS[1],
        files: &[file!("flowchart.rhizo", "playground/02-flowchart.rhizo")],
    },
    Sample {
        id: "checkout-sequence",
        title: "Checkout",
        blurb: "Who calls whom, in order",
        group: GROUPS[1],
        files: &[file!("checkout-sequence.rhizo", "checkout-sequence.rhizo")],
    },
    Sample {
        id: "many-frames",
        title: "Launch planning",
        blurb: "A layout per frame",
        group: GROUPS[1],
        files: &[file!("many-frames.rhizo", "playground/06-many-frames.rhizo")],
    },
    Sample {
        id: "data-model",
        title: "Shop data model",
        blurb: "Compiles to OpenAPI, AsyncAPI",
        group: GROUPS[2],
        files: &[file!("data-model.rhizo", "playground/11-data-model.rhizo")],
    },
    Sample {
        id: "aws-infra",
        title: "Shop on AWS",
        blurb: "Compiles to OpenTofu",
        group: GROUPS[3],
        files: &[file!("aws-infra.rhizo", "playground/07-typed-infra.rhizo")],
    },
    Sample {
        id: "azure-infra",
        title: "Shop on Azure",
        blurb: "With Azure Verified Modules",
        group: GROUPS[3],
        files: &[file!("azure-infra.rhizo", "playground/12-azure-infra.rhizo")],
    },
    Sample {
        id: "gcp-infra",
        title: "Shop on Google Cloud",
        blurb: "Cloud Run and API Gateway",
        group: GROUPS[3],
        files: &[file!("gcp-infra.rhizo", "playground/13-gcp-infra.rhizo")],
    },
    Sample {
        id: "c4-system",
        title: "C4 system",
        blurb: "Context, containers, parts",
        group: GROUPS[3],
        files: &[file!("c4-system.rhizo", "c4-system.rhizo")],
    },
    Sample {
        id: "shop",
        title: "Shop project",
        blurb: "Six files, linked to each other",
        group: GROUPS[4],
        files: &[
            file!("model.rhizo", "shop/model.rhizo"),
            file!("intent.rhizo", "shop/intent.rhizo"),
            file!("api.rhizo", "shop/api.rhizo"),
            file!("infra.rhizo", "shop/infra.rhizo"),
            file!("flows/checkout.rhizo", "shop/flows/checkout.rhizo"),
            file!("flows/order-intake.rhizo", "shop/flows/order-intake.rhizo"),
        ],
    },
    Sample {
        id: "mermaid",
        title: "Mermaid flowchart",
        blurb: "Opening it imports it",
        group: GROUPS[5],
        files: &[file!("checkout.mmd", "playground/09-checkout.mmd")],
    },
    Sample {
        id: "outline",
        title: "Markdown outline",
        blurb: "Lists become a mindmap",
        group: GROUPS[5],
        files: &[file!("reading-list.md", "playground/10-outline.md")],
    },
];

pub fn find(id: &str) -> Option<&'static Sample> {
    SAMPLES.iter().find(|s| s.id == id)
}

/// Writes a copy of `sample` and returns what to open: a project goes into
/// the folder `to` (new or empty); a file goes to `to`, or into it when
/// `to` is a folder. Never overwrites a file.
pub fn copy(sample: &Sample, to: &Path) -> Result<PathBuf, String> {
    let exists = |p: &Path| format!("{} exists: choose a new name", p.display());
    if sample.is_project() {
        if to.is_file() || to.read_dir().is_ok_and(|mut d| d.next().is_some()) {
            return Err(format!("{} exists and is not empty", to.display()));
        }
        for (rel, src) in sample.files {
            let path = to.join(rel);
            std::fs::create_dir_all(path.parent().unwrap_or(to)).map_err(|e| format!("cannot create {e}"))?;
            write_new(&path, src).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        }
        return Ok(to.to_path_buf());
    }
    let (rel, src) = sample.files[0];
    let path = if to.is_dir() { to.join(rel) } else { to.to_path_buf() };
    if path.exists() {
        return Err(exists(&path));
    }
    write_new(&path, src).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok(path)
}

fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::OpenOptions::new().write(true).create_new(true).open(path)?.write_all(text.as_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Project;
    use rhizo_kit::Kits;

    /// Every sample loads and checks without errors, from a copy on disk,
    /// as `rhizo check` would; imports are followed in the project.
    #[test]
    fn every_sample_parses_and_checks() {
        let kits = Kits::builtin();
        let dir = std::env::temp_dir().join(format!("rhizo-samples-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        let mut ids: Vec<&str> = SAMPLES.iter().map(|s| s.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), SAMPLES.len(), "ids are unique");
        for s in SAMPLES {
            assert!(GROUPS.contains(&s.group), "{}: group {}", s.id, s.group);
            assert!(
                !s.title.is_empty() && !s.blurb.is_empty() && s.blurb.chars().count() <= 32,
                "{}: a short line",
                s.id
            );
            let to = if s.is_project() { dir.join(s.id) } else { dir.clone() };
            std::fs::create_dir_all(&dir).unwrap();
            let opened = copy(s, &to).unwrap();
            let p = Project::open(&opened).unwrap_or_else(|e| panic!("{}: {e}", s.id));
            assert_eq!(p.files.len(), s.files.len(), "{}: every file loads", s.id);
            let errors: Vec<_> = p.check(&kits).into_iter().flatten().filter(|d| d.is_error()).collect();
            assert!(errors.is_empty(), "{}: {errors:#?}", s.id);
            assert!(p.files.iter().all(|f| !f.doc.nodes.is_empty()), "{}: something to see", s.id);
            assert!(copy(s, &to).is_err(), "{}: a copy never overwrites", s.id);
        }
        // Every group has a sample.
        for g in GROUPS {
            assert!(SAMPLES.iter().any(|s| s.group == *g), "{g}");
        }
        assert_eq!(find("shop").map(Sample::is_project), Some(true));
        std::fs::remove_dir_all(&dir).ok();
    }
}
