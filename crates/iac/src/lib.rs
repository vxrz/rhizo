//! Importing OpenTofu code into a rhizo diagram (docs/design/iac-import.md,
//! phase 1: verbatim import with the L1 check).
//!
//! [`import_tofu`] reads a root module, plans it offline with OpenTofu, and
//! writes one `infra.environment` frame: an `infra.resource` or
//! `infra.module` node per resource, data source and module call, every
//! other block verbatim on the frame, and a dependency edge per reference.
//! Nodes whose code rhizo recognizes are folded into neutral types (phase 2):
//! still verbatim, with the props their literal arguments state, typed
//! edges and nesting.
//! Expressions are stored as the HCL text they were written as, so nothing
//! is re-interpreted. Then L1 compiles the diagram and checks that OpenTofu
//! plans the result exactly like the original. Anything that would be lost
//! fails the import, naming the construct.
//!
//! OpenTofu runs with argument lists (no shell), in temporary copies (the
//! input folder is never written), against an empty local state, with every provider
//! mocked and without the caller's cloud settings. State is never read.

mod adopt;
mod diagram;
mod hcl;
mod l1;
mod mocks;
mod providers;
mod tofu;

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::Sender;

use rhizo_core::Document;

pub use adopt::{Adopted, adopt, adoptable, verify_adopted};
pub use providers::{ALLOWED, parse_allowed, short as short_provider};

#[derive(Clone, Debug)]
pub struct Options {
    /// Run L1 after importing (the default).
    pub verify: bool,
    /// The `tofu` executable.
    pub tofu: PathBuf,
    /// Where providers are cached between runs. Default: the
    /// `TF_PLUGIN_CACHE_DIR` variable, else one temporary cache per run.
    pub plugin_cache: Option<PathBuf>,
    /// Set to stop: the running `tofu` is killed and [`Error::Cancelled`]
    /// returned (a viewer's background adopt).
    pub cancel: Option<Arc<AtomicBool>>,
    /// Receives what a long run is doing ("planning the original code").
    pub progress: Option<Sender<String>>,
    /// Providers the person allows for this run besides [`ALLOWED`]:
    /// `NAMESPACE/NAME` or `HOST/NAMESPACE/NAME` ([`parse_allowed`]).
    pub allow_providers: Vec<String>,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            verify: true,
            tofu: PathBuf::from("tofu"),
            plugin_cache: None,
            cancel: None,
            progress: None,
            allow_providers: Vec::new(),
        }
    }
}

impl Options {
    /// Says what the run does next, to whoever listens.
    pub(crate) fn step(&self, what: &str) {
        if let Some(p) = &self.progress {
            let _ = p.send(what.to_string());
        }
    }
}

/// A finished import.
#[derive(Debug)]
pub struct Imported {
    /// The diagram, in canonical rhizo text.
    pub kdl: String,
    /// What each construct became, and the L1 result.
    pub report: String,
}

#[derive(Debug)]
pub enum Error {
    /// Something would be lost or cannot be read; each entry names it.
    Refused(Vec<String>),
    /// OpenTofu failed at `step`.
    Tofu {
        step: String,
        output: String,
    },
    /// L1 found differences between the original and the compiled code.
    Mismatch(Vec<String>),
    /// A node cannot be adopted, or adopting would lose what the original
    /// code manages; each entry names it.
    NotAdopted(Vec<String>),
    Io(String),
    /// The code requires providers that are not allowed: each with where it
    /// is required. Nothing was installed or run.
    NotAllowed(Vec<(String, String)>),
    /// Stopped through [`Options::cancel`]; nothing was written.
    Cancelled,
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let list = |f: &mut fmt::Formatter<'_>, items: &[String]| items.iter().try_for_each(|i| write!(f, "\n  - {i}"));
        match self {
            Error::Refused(items) => {
                write!(f, "refusing to import: it would not be lossless")?;
                list(f, items)
            }
            Error::NotAdopted(items) => {
                write!(f, "refusing to adopt: nothing was written")?;
                list(f, items)
            }
            Error::Tofu { step, output } => write!(f, "{step} failed:\n{output}"),
            Error::Mismatch(items) => {
                write!(f, "L1 failed: the compiled code is not the same as the original")?;
                list(f, items)
            }
            Error::Io(e) => write!(f, "{e}"),
            Error::NotAllowed(found) => {
                write!(f, "refusing to run OpenTofu: the code requires providers rhizo does not install by default")?;
                for (p, at) in found {
                    write!(f, "\n  - {}, required by {at}", providers::short(p))?;
                }
                let mut names: Vec<&str> = found.iter().map(|(p, _)| providers::short(p)).collect();
                names.sort_unstable();
                names.dedup();
                write!(
                    f,
                    "\nOpenTofu starts every provider the code requires, even mocked, and a provider is a program from its publisher that runs on this machine. Nothing was installed. If you trust {}, allow {} for one run: {} (CLI), `allow_providers` (MCP, only with the person's approval) or \"Allow and adopt\" (viewer).",
                    if names.len() == 1 { "it" } else { "them" },
                    if names.len() == 1 { "it" } else { "them" },
                    names.iter().map(|n| format!("`--allow-provider {n}`")).collect::<Vec<_>>().join(" "),
                )
            }
            Error::Cancelled => write!(f, "cancelled: nothing was written"),
        }
    }
}

impl std::error::Error for Error {}

/// The name a folder goes by in the diagram: its own name, even for `.`.
fn folder_name(dir: &Path) -> Result<String, Error> {
    let abs = dir.canonicalize().map_err(|e| Error::Io(format!("{}: {e}", dir.display())))?;
    Ok(abs.file_name().map_or_else(|| "root".into(), |n| n.to_string_lossy().into_owned()))
}

/// Imports the root module in `dir`.
pub fn import_tofu(dir: &Path, opts: &Options) -> Result<Imported, Error> {
    let name = folder_name(dir)?;
    let src = hcl::Source::read(dir).map_err(Error::Refused)?;
    let run = tofu::Run::new(opts)?;
    let copy = run.workdir("original")?;
    tofu::copy_tree(dir, &copy)?;
    let planned = run.plan(&copy, &src.blocks, "original")?;
    let (file, placed, summary) = diagram::build(&name, &src, &planned).map_err(Error::Refused)?;
    let kdl = rhizo_lang::write::write(&file);
    let doc = reload(&kdl, &file)?;
    let mut report = format!("rhizo import tofu {}\n", dir.display());
    let width = placed.iter().map(|(what, _)| what.chars().count()).max().unwrap_or(0);
    for (what, to) in &placed {
        report += &format!("  {what:<width$}  -> {to}\n");
    }
    for name in &src.inputs {
        report += &format!(
            "  {name:<width$}  -- not imported: variable values (may be secret); L1 uses them on both sides\n"
        );
    }
    for (name, why) in &src.skipped {
        report += &format!("  {name:<width$}  -- not imported: {why}\n");
    }
    report += &format!("  {:<width$}  -- not carried: comments do not change the plan\n", "comments");
    for line in &summary {
        report += &format!("{line}\n");
    }
    if opts.verify {
        let s = l1::verify(&run, &doc, dir, &src, &planned)?;
        report += &format!(
            "L1 equal: {} blocks, {} planned resource instances and {} outputs are the same after compiling\n",
            s.blocks, s.instances, s.outputs
        );
    } else {
        report += "L1 not run (--no-verify)\n";
    }
    Ok(Imported { kdl, report })
}

/// L1 for an existing diagram: its imported environment compiles to code
/// that plans like the root module in `dir`. Returns the summary line.
pub fn verify_diagram(dir: &Path, kdl: &str, opts: &Options) -> Result<String, Error> {
    let doc = rhizo_lang::load(kdl, rhizo_lang::Syntax::Kdl).into_result().map_err(|d| {
        Error::Refused(d.into_iter().map(|d| format!("the diagram does not load: {}", d.message)).collect())
    })?;
    let src = hcl::Source::read(dir).map_err(Error::Refused)?;
    let run = tofu::Run::new(opts)?;
    let copy = run.workdir("original")?;
    tofu::copy_tree(dir, &copy)?;
    let planned = run.plan(&copy, &src.blocks, "original")?;
    let s = l1::verify(&run, &doc.0, dir, &src, &planned)?;
    Ok(format!("L1 equal: {} blocks, {} planned resource instances and {} outputs", s.blocks, s.instances, s.outputs))
}

/// Loads the written diagram back and checks it holds exactly what was
/// built (a value KDL cannot carry would show here) and passes the kit.
fn reload(kdl: &str, file: &rhizo_lang::ast::File) -> Result<Document, Error> {
    let internal = |m: String| Error::Refused(vec![m]);
    let (doc, _) = rhizo_lang::load(kdl, rhizo_lang::Syntax::Kdl)
        .into_result()
        .map_err(|d| internal(format!("the written diagram does not load: {}", d[0].message)))?;
    for f in &file.frames {
        let Some(frame) = doc.frames.iter().find(|x| Some(x.id.as_str()) == f.id.as_deref()) else {
            return Err(internal("the written diagram lost its frame".into()));
        };
        if frame.props != f.props {
            return Err(internal(format!("frame `{}`: its blocks do not survive the diagram's text form", frame.id)));
        }
        let mut stack: Vec<&rhizo_lang::ast::NodeDef> = f.nodes.iter().collect();
        while let Some(n) = stack.pop() {
            let id = n.id.as_deref().unwrap_or_default();
            if doc.nodes.get(&rhizo_core::NodeId::from(id)).map(|x| &x.props) != Some(&n.props) {
                return Err(internal(format!("{id}: its props do not survive the diagram's text form")));
            }
            stack.extend(&n.children);
        }
    }
    let errors: Vec<String> = rhizo_kit::check(&doc, &rhizo_kit::Kits::builtin())
        .into_iter()
        .filter(|f| f.diagnostic.is_error())
        .map(|f| format!("the diagram fails its kit check: {}", f.diagnostic.message))
        .collect();
    if errors.is_empty() { Ok(doc) } else { Err(Error::Refused(errors)) }
}
