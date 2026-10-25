//! `rhizo`: render, convert, check and format diagrams.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::{Parser, Subcommand};
use miette::{IntoDiagnostic, Result, miette};
use rhizo_core::{Document, Theme};
use rhizo_interop::Format;
use rhizo_kit::Kits;
use rhizo_lang::{Diagnostic, Report};
use rhizo_project::Project;
use rhizo_render::{Fonts, SvgOptions};

#[derive(Parser)]
#[command(name = "rhizo", version, about = "Diagrams and mindmaps as text. Render, convert, check, format.")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Args, Clone)]
struct RenderArgs {
    /// Theme: light, dark or paper. Overrides the document's `theme`.
    #[arg(long)]
    theme: Option<String>,
    /// Pixel density for PNG output.
    #[arg(long, default_value_t = 2.0)]
    scale: f32,
    /// No background (SVG and PNG).
    #[arg(long)]
    transparent: bool,
}

#[derive(Subcommand)]
enum Command {
    /// Render to SVG, PNG or PDF.
    Render {
        input: PathBuf,
        /// Output file; the extension picks the format. Defaults to INPUT.svg.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Output format, if the extension does not say (svg, png, pdf).
        #[arg(short, long)]
        format: Option<String>,
        #[command(flatten)]
        render: RenderArgs,
    },
    /// Convert between any supported formats (see `rhizo formats`).
    Convert {
        input: PathBuf,
        /// Output file, or `-` for stdout.
        #[arg(short, long)]
        output: PathBuf,
        /// Input format, if the extension does not say.
        #[arg(long)]
        from: Option<String>,
        /// Output format, if the extension does not say.
        #[arg(long)]
        to: Option<String>,
        #[command(flatten)]
        render: RenderArgs,
    },
    /// Report problems without writing anything, including props and ports
    /// that break the rules of the kits a document uses.
    Check {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
    },
    /// Rewrite rhizo files in canonical form.
    Fmt {
        #[arg(required = true)]
        inputs: Vec<PathBuf>,
        /// Fail if any file would change, instead of rewriting it.
        #[arg(long)]
        check: bool,
    },
    /// Re-render whenever the input changes.
    Watch {
        input: PathBuf,
        #[arg(short, long)]
        output: Option<PathBuf>,
        #[command(flatten)]
        render: RenderArgs,
    },
    /// Print a starter document, or write a starter project into a folder.
    /// Kinds: mindmap, flow, canvas, data, sequence, workflow, infra, intent,
    /// learn (a hands-on tutorial for the viewer: `rhizo new learn >
    /// learn.rhizo && rhizo-view learn.rhizo`); `project DIR` writes a folder
    /// of files that refer to each other.
    New {
        #[arg(default_value = "mindmap")]
        kind: String,
        /// For `project`: the folder to create.
        dir: Option<PathBuf>,
    },
    /// List the built-in samples (the repo's examples), or copy one:
    /// `rhizo samples copy shop ./shop`. The viewer has them under File ▸
    /// Open Sample….
    Samples {
        #[command(subcommand)]
        action: Option<SamplesAction>,
    },
    /// Print the JSON Schema for the JSON/YAML forms.
    Schema,
    /// List supported formats.
    Formats,
    /// Compile a typed document into a bundle of specs (JSON Schema,
    /// OpenAPI) plus manifest.json.
    Compile {
        input: PathBuf,
        /// Bundle folder. Files rhizo generated there before and no longer
        /// produces are removed; files it did not generate are never touched.
        #[arg(short, long, default_value = "build")]
        output: PathBuf,
        /// Fail if the bundle is out of date, instead of writing it.
        #[arg(long)]
        check: bool,
    },
    /// Print the build brief: the goal, decisions, generated specs and the
    /// ordered tasks an agent needs to build what the diagram describes.
    Brief {
        input: PathBuf,
        /// tasks.json instead of Markdown.
        #[arg(long)]
        json: bool,
    },
    /// Serve a document, or a project folder, to AI agents over the Model
    /// Context Protocol (stdio).
    /// Read-only: agents explore, check and preview; changes they propose
    /// land in .rhizo/proposals/ for you to apply.
    Mcp {
        input: PathBuf,
        /// Let agents write the document (validated; the viewer updates live).
        /// The file may then not exist yet.
        #[arg(long)]
        allow_write: bool,
        /// Let `adopt_nodes` install and run this OpenTofu provider besides
        /// rhizo's allow-list (NAMESPACE/NAME or HOST/NAMESPACE/NAME;
        /// repeatable). Agents cannot add providers; only this flag can.
        #[arg(long = "allow-provider", value_name = "NAMESPACE/NAME", value_parser = rhizo_iac::parse_allowed)]
        allow_providers: Vec<String>,
    },
    /// Print the guide to writing rhizo diagrams, for people and language
    /// models (syntax, layouts, kits, goals and decisions).
    Guide,
    /// Who makes rhizo, its licence and where the code lives.
    About,
    /// List, show, apply or reject the changes agents proposed for a project
    /// (stored in `.rhizo/proposals/`). Applying checks the change still fits
    /// and leaves no errors.
    Proposals {
        /// The project folder (or a file in it). Default: the current folder.
        #[arg(default_value = ".")]
        project: PathBuf,
        #[arg(long, value_name = "N")]
        show: Option<u32>,
        #[arg(long, value_name = "N", conflicts_with_all = ["show", "reject"])]
        apply: Option<u32>,
        #[arg(long, value_name = "N", conflicts_with = "show")]
        reject: Option<u32>,
    },
    /// List kits and their types, or show one type with its props schema.
    Types {
        /// A kit (`data`) or a type (`data.field`).
        name: Option<String>,
    },
    /// Import existing infrastructure code as a diagram. `rhizo import tofu
    /// DIR -o FILE` reads an OpenTofu/Terraform root module, writes it as an
    /// infra environment, and proves with OpenTofu (offline, mocked
    /// providers, empty state) that compiling the diagram gives the same
    /// plan. Anything that would be lost makes it fail.
    Import {
        /// What to import from (only `tofu`).
        #[arg(value_enum)]
        from: ImportFrom,
        /// The root module's folder. It is never written.
        dir: PathBuf,
        /// The diagram to write.
        #[arg(short, long)]
        output: PathBuf,
        /// Write the import report here instead of stdout.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Skip the L1 check (compile and compare the plans).
        #[arg(long)]
        no_verify: bool,
        /// Also install and run this provider for this run (OpenTofu runs
        /// every provider the code requires, even mocked): NAMESPACE/NAME or
        /// HOST/NAMESPACE/NAME; repeatable. Only for providers you trust.
        #[arg(long = "allow-provider", value_name = "NAMESPACE/NAME", value_parser = rhizo_iac::parse_allowed)]
        allow_providers: Vec<String>,
    },
    /// Switch imported nodes to rhizo's own code, so their props become
    /// editable. Rewrites FILE in place: each node loses its verbatim code
    /// and keeps where it came from, which compiles to `moved` blocks.
    /// Nothing is written unless L1-adopt passes: the code before and after
    /// planned offline (mocked providers, empty state) must account for
    /// every original instance; changed attributes and new instances are
    /// listed in the report.
    Adopt {
        /// The diagram with the imported environment.
        file: PathBuf,
        /// Node ids to adopt (their addresses, e.g. `module.receipts`).
        nodes: Vec<String>,
        /// Adopt every node that switches to rhizo's code without a change; the report names the others.
        #[arg(long, conflicts_with = "nodes")]
        all: bool,
        /// Write the adopt report here instead of stdout.
        #[arg(long)]
        report: Option<PathBuf>,
        /// Variable values both plans read (a `.tfvars` file); repeatable.
        #[arg(long = "var-file")]
        var_files: Vec<PathBuf>,
        /// Compare FILE with this earlier version of it (L1-adopt only, for
        /// adopted nodes changed since): writes nothing.
        #[arg(long, conflicts_with_all = ["nodes", "all"])]
        against: Option<PathBuf>,
        /// Run the shape checks and L1-adopt and print the report, but write nothing.
        #[arg(long, conflicts_with = "against")]
        dry_run: bool,
        /// Also install and run this provider for this run (OpenTofu runs
        /// every provider the code requires, even mocked): NAMESPACE/NAME or
        /// HOST/NAMESPACE/NAME; repeatable. Only for providers you trust.
        #[arg(long = "allow-provider", value_name = "NAMESPACE/NAME", value_parser = rhizo_iac::parse_allowed)]
        allow_providers: Vec<String>,
    },
    /// Check that infrastructure code survives a round trip through a
    /// diagram: import DIR and compare the plans (L1). With --diagram,
    /// compare that diagram's imported environment with DIR instead (for
    /// CI: code and diagram still agree).
    VerifyRoundtrip {
        dir: PathBuf,
        #[arg(long)]
        diagram: Option<PathBuf>,
        /// Also install and run this provider for this run (OpenTofu runs
        /// every provider the code requires, even mocked): NAMESPACE/NAME or
        /// HOST/NAMESPACE/NAME; repeatable. Only for providers you trust.
        #[arg(long = "allow-provider", value_name = "NAMESPACE/NAME", value_parser = rhizo_iac::parse_allowed)]
        allow_providers: Vec<String>,
    },
}

#[derive(Subcommand)]
enum SamplesAction {
    /// Write a copy of sample ID: a file to PATH (or into PATH when it is a
    /// folder), a project into the new or empty folder PATH. Never overwrites.
    Copy { id: String, path: PathBuf },
}

#[derive(Clone, Copy, clap::ValueEnum)]
enum ImportFrom {
    /// OpenTofu or Terraform code (`tofu` must be installed).
    Tofu,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Render { input, output, format, render } => {
            let output = output.unwrap_or_else(|| input.with_extension(format.as_deref().unwrap_or("svg")));
            let doc = load(&input, None)?.drawable(0, &Kits::builtin());
            let to = resolve_format(&output, format.as_deref())?;
            if !to.is_image() {
                return Err(miette!("`render` writes svg, png or pdf; use `rhizo convert` for {}", to.name()));
            }
            write_output(&output, &produce(&doc, to, &render)?)?;
            report_written(&output);
        }
        Command::Convert { input, output, from, to, render } => {
            let p = load(&input, from.as_deref())?;
            let to = resolve_format(&output, to.as_deref())?;
            // References into imports are drawn as their targets; rhizo text keeps them as written.
            let doc = if to.syntax().is_some() { p.files[0].doc.clone() } else { p.drawable(0, &Kits::builtin()) };
            write_output(&output, &produce(&doc, to, &render)?)?;
            report_written(&output);
        }
        Command::Check { inputs } => {
            let kits = Kits::builtin();
            let mut failed = 0;
            for input in &inputs {
                let p = match open(input) {
                    Ok(p) => p,
                    // Unreadable files count as failed; folders and formats rhizo cannot read stop here.
                    Err(e) if !input.is_dir() && input_format(input, None).is_ok() => {
                        eprintln!("{e:?}");
                        failed += 1;
                        continue;
                    }
                    Err(e) => return Err(e),
                };
                let (m, lone) = (&p.merged, lone(input, &p));
                match if lone { check_lone(input, &p, &kits) } else { report_project(&p, &kits) } {
                    Ok(()) if lone => {
                        eprintln!("✓ {} ({} nodes, {} edges)", input.display(), m.nodes.len(), m.edges.len())
                    }
                    Ok(()) => eprintln!(
                        "✓ {} ({} files, {} nodes, {} edges)",
                        input.display(),
                        p.files.len(),
                        m.nodes.len(),
                        m.edges.len()
                    ),
                    Err(e) => {
                        eprintln!("{e:?}");
                        failed += 1;
                    }
                }
            }
            if failed > 0 {
                std::process::exit(1);
            }
        }
        Command::Fmt { inputs, check } => {
            let mut changed = 0;
            for input in &inputs {
                if input_format(input, None)? != Format::Rhizo {
                    return Err(miette!("{} is not a rhizo file", input.display()));
                }
                let p = load(input, None)?;
                let (src, formatted) = (&p.files[0].src, rhizo_lang::to_kdl(&p.files[0].doc));
                if formatted == *src {
                    continue;
                }
                changed += 1;
                if check {
                    eprintln!("would reformat {}", input.display());
                } else if input.as_os_str() == "-" {
                    print!("{formatted}");
                } else {
                    std::fs::write(input, formatted).into_diagnostic()?;
                    eprintln!("formatted {}", input.display());
                }
            }
            if check && changed > 0 {
                std::process::exit(1);
            }
        }
        Command::Watch { input, output, render } => watch(&input, output, &render)?,
        Command::New { kind, dir } if kind == "project" => {
            let dir = dir.ok_or_else(|| miette!("say where: rhizo new project my-project"))?;
            if dir.exists() && std::fs::read_dir(&dir).into_diagnostic()?.next().is_some() {
                return Err(miette!("{} exists and is not empty", dir.display()));
            }
            for (rel, src) in rhizo_lang::starters::PROJECT {
                let path = dir.join(rel);
                std::fs::create_dir_all(path.parent().unwrap_or(&dir)).into_diagnostic()?;
                std::fs::write(&path, src).map_err(|e| miette!("cannot write {}: {e}", path.display()))?;
            }
            eprintln!(
                "wrote {} ({} files). Next: rhizo-view {0} · rhizo check {0} · rhizo brief {0}",
                dir.display(),
                rhizo_lang::starters::PROJECT.len()
            );
        }
        Command::New { kind, .. } => {
            let src = rhizo_lang::starters::starter(&kind).ok_or_else(|| {
                let kinds: Vec<&str> = rhizo_lang::starters::STARTERS
                    .iter()
                    .chain(rhizo_lang::starters::MORE)
                    .map(|(k, _)| *k)
                    .chain(["project"])
                    .collect();
                miette!("unknown starter `{kind}`: try {}", kinds.join(", "))
            })?;
            print!("{src}")
        }
        Command::Samples { action: None } => {
            use rhizo_project::samples::{GROUPS, SAMPLES};
            for g in GROUPS {
                println!("{g}");
                for s in SAMPLES.iter().filter(|s| s.group == *g) {
                    println!("  {:<18} {:<22} {}", s.id, s.title, s.blurb);
                }
            }
            eprintln!("copy one: rhizo samples copy ID PATH");
        }
        Command::Samples { action: Some(SamplesAction::Copy { id, path }) } => {
            use rhizo_project::samples;
            let s = samples::find(&id).ok_or_else(|| {
                let ids: Vec<&str> = samples::SAMPLES.iter().map(|s| s.id).collect();
                miette!("no sample `{id}`: try {}", ids.join(", "))
            })?;
            let wrote = samples::copy(s, &path).map_err(|e| miette!("{e}"))?;
            eprintln!("wrote {}. Next: rhizo-view {0}", wrote.display());
        }
        Command::Schema => {
            println!("{}", serde_json::to_string_pretty(&rhizo_lang::json_schema()).into_diagnostic()?)
        }
        Command::Types { name } => types(name.as_deref())?,
        Command::Proposals { project, show, apply, reject } => {
            use rhizo_project::proposals;
            let root = if project.is_dir() {
                project.clone()
            } else {
                project.parent().map(Path::to_path_buf).unwrap_or_default()
            };
            if let Some(n) = apply {
                let path = proposals::apply(&root, n, &Kits::builtin()).map_err(|e| miette!("{e}"))?;
                eprintln!("applied proposal {n} to {}", path.display());
            } else if let Some(n) = reject {
                proposals::reject(&root, n).map_err(|e| miette!("{e}"))?;
                eprintln!("rejected proposal {n}");
            } else if let Some(n) = show {
                let p =
                    proposals::list(&root).into_iter().find(|p| p.id == n).ok_or_else(|| miette!("no proposal {n}"))?;
                print!("{}", proposals::markdown(&p));
            } else {
                let all = proposals::list(&root);
                if all.is_empty() {
                    eprintln!("no proposals in {}", root.join(".rhizo/proposals").display());
                }
                for p in all {
                    let status = format!("{:?}", p.status).to_lowercase();
                    let reason = p.rationale.lines().next().unwrap_or("");
                    println!("{:>4}  {status:<8} {:<28} {reason}", p.id, p.file);
                }
            }
        }
        Command::Brief { input, json } => {
            let mut bundle = bundle_for(&input, &Kits::builtin())?;
            let key = if json { rhizo_compile::TASKS } else { rhizo_compile::AGENTS };
            print!("{}", bundle.files.remove(key).map(|f| f.contents).unwrap_or_default());
        }
        Command::Mcp { input, allow_write, allow_providers } => {
            if !input.is_dir() {
                input_format(&input, None)?;
            }
            if !input.exists() && !allow_write {
                return Err(miette!(
                    "{} is not a file (pass --allow-write to let an agent create it)",
                    input.display()
                ));
            }
            let allow_providers = iac_options(allow_providers).allow_providers;
            rhizo_mcp::serve_stdio(input, allow_write, allow_providers).map_err(|e| miette!("mcp: {e}"))?
        }
        Command::Guide => print!("{}", Kits::builtin().guide()),
        Command::About => {
            for line in rhizo_core::about::footer_lines(rhizo_core::about::GITHUB) {
                // "About rhizo" is a link in the window; here it is what this prints.
                println!("{}", line.trim_end_matches(" · About rhizo"));
            }
        }
        Command::Compile { input, output, check } => compile(&input, &output, check)?,
        Command::Import { from: ImportFrom::Tofu, dir, output, report, no_verify, allow_providers } => {
            let opts = rhizo_iac::Options { verify: !no_verify, ..iac_options(allow_providers) };
            let imported = rhizo_iac::import_tofu(&dir, &opts).map_err(|e| miette!("{e}"))?;
            std::fs::write(&output, &imported.kdl).map_err(|e| miette!("cannot write {}: {e}", output.display()))?;
            match report {
                Some(r) => {
                    std::fs::write(&r, &imported.report).map_err(|e| miette!("cannot write {}: {e}", r.display()))?
                }
                None => print!("{}", imported.report),
            }
            report_written(&output);
        }
        Command::Adopt { file, nodes, all, report, var_files, against, dry_run, allow_providers } => {
            let opts = iac_options(allow_providers);
            let src = read_input(&file)?;
            let text = match against {
                Some(old) => rhizo_iac::verify_adopted(&read_input(&old)?, &src, &opts, &var_files),
                None if nodes.is_empty() && !all => {
                    return Err(miette!("name the nodes to adopt, or pass --all"));
                }
                None => rhizo_iac::adopt(&src, &nodes, all, &opts, &var_files).and_then(|a| {
                    if dry_run {
                        return Ok(format!("{}(dry run: {} not written)\n", a.report, file.display()));
                    }
                    std::fs::write(&file, &a.kdl)
                        .map_err(|e| rhizo_iac::Error::Io(format!("cannot write {}: {e}", file.display())))?;
                    report_written(&file);
                    Ok(a.report)
                }),
            }
            .map_err(|e| miette!("{e}"))?;
            match report {
                Some(r) => std::fs::write(&r, &text).map_err(|e| miette!("cannot write {}: {e}", r.display()))?,
                None => print!("{text}"),
            }
        }
        Command::VerifyRoundtrip { dir, diagram, allow_providers } => {
            let opts = iac_options(allow_providers);
            let summary = match diagram {
                Some(d) => rhizo_iac::verify_diagram(&dir, &read_input(&d)?, &opts),
                None => rhizo_iac::import_tofu(&dir, &opts).map(|i| i.report),
            }
            .map_err(|e| miette!("{e}"))?;
            println!("{}", summary.trim_end());
        }
        Command::Formats => {
            println!("{:<10} {:<6} {:<6} extensions", "format", "read", "write");
            for f in Format::ALL {
                let mark = |b: bool| if b { "yes" } else { "" };
                println!("{:<10} {:<6} {:<6} {}", f.name(), mark(f.can_import()), "yes", f.extensions().join(", "));
            }
        }
    }
    Ok(())
}

fn resolve_format(path: &Path, explicit: Option<&str>) -> Result<Format> {
    match explicit {
        Some(name) => Format::from_name(name).ok_or_else(|| miette!("unknown format `{name}` (see `rhizo formats`)")),
        None => Format::from_path(path).ok_or_else(|| {
            miette!("cannot tell the format of `{}` from its extension; pass --to or --format", path.display())
        }),
    }
}

fn input_format(path: &Path, explicit: Option<&str>) -> Result<Format> {
    let f = match explicit {
        Some(_) => resolve_format(path, explicit)?,
        None if path.as_os_str() == "-" => Format::Rhizo,
        None => resolve_format(path, None)?,
    };
    if !f.can_import() {
        return Err(miette!("rhizo cannot read {} files", f.name()));
    }
    Ok(f)
}

fn read_input(path: &Path) -> Result<String> {
    if path.as_os_str() == "-" {
        let mut s = String::new();
        std::io::stdin().read_to_string(&mut s).into_diagnostic()?;
        return Ok(s);
    }
    std::fs::read_to_string(path).map_err(|e| miette!("cannot read {}: {e}", path.display()))
}

/// A file (or stdin) and what it imports, read as `from` or by its
/// extension. Nothing is reported yet.
fn open_file(path: &Path, from: Option<&str>) -> Result<Project> {
    let format = input_format(path, from)?;
    let src = read_input(path)?;
    Ok(if path.as_os_str() == "-" {
        Project::open_text("-", &src, format)
    } else {
        Project::with_text(path, src, format)
    })
}

/// A folder as a project, or a file as [`open_file`] opens it.
fn open(path: &Path) -> Result<Project> {
    if path.is_dir() {
        return Project::load_dir(path).map_err(|e| miette!("{e}"));
    }
    open_file(path, None)
}

/// [`open_file`], then [`own_problems`].
fn load(path: &Path, from: Option<&str>) -> Result<Project> {
    let p = open_file(path, from)?;
    own_problems(path, &p)?;
    Ok(p)
}

/// Prints the opened file's own warnings; fails if it has errors.
fn own_problems(path: &Path, p: &Project) -> Result<()> {
    let (name, file) = (path.display().to_string(), &p.files[0]);
    if file.has_errors() {
        return Err(failure(&name, &file.src, file.load.clone()));
    }
    print_diagnostics(&name, &file.src, file.load.clone());
    Ok(())
}

/// One file that imports nothing: reported on its own, not as a project.
fn lone(path: &Path, p: &Project) -> bool {
    !path.is_dir() && p.is_single() && (p.files[0].doc.imports.is_empty() || path.as_os_str() == "-")
}

fn failure(name: &str, src: &str, mut errors: Vec<Diagnostic>) -> miette::Report {
    errors.sort_by_key(|d| !d.is_error());
    let hidden = errors.len().saturating_sub(MAX_SHOWN);
    errors.truncate(MAX_SHOWN);
    if hidden > 0 {
        eprintln!("(showing {MAX_SHOWN} of {} problems)", MAX_SHOWN + hidden);
    }
    Report::new(name, src, errors).into()
}

/// A lone file's problems: its own, then the kits'; fails on errors.
fn check_lone(path: &Path, p: &Project, kits: &Kits) -> Result<()> {
    own_problems(path, p)?;
    let (name, file) = (path.display().to_string(), &p.files[0]);
    let kit = p.check(kits).swap_remove(0).split_off(file.load.len());
    if kit.iter().any(Diagnostic::is_error) {
        return Err(failure(&name, &file.src, kit));
    }
    print_diagnostics(&name, &file.src, kit);
    Ok(())
}

/// Prints every file's problems; fails if any is an error.
fn report_project(p: &rhizo_project::Project, kits: &Kits) -> Result<()> {
    let mut errors = 0;
    for (f, diags) in p.check(kits).into_iter().enumerate() {
        let file = &p.files[f];
        let name = file.path.display().to_string();
        let n = diags.iter().filter(|d| d.is_error()).count();
        errors += n;
        if n > 0 {
            eprintln!("{:?}", failure(&name, &file.src, diags));
        } else {
            print_diagnostics(&name, &file.src, diags);
        }
    }
    let place = if p.root.as_os_str().is_empty() { Path::new(".") } else { p.root.as_path() };
    if errors > 0 { Err(miette!("{errors} error(s) in {}", place.display())) } else { Ok(()) }
}

/// Compiles a file, or a folder or a file with imports as one project.
fn bundle_for(input: &Path, kits: &Kits) -> Result<rhizo_compile::Bundle> {
    let p = open(input)?;
    if !lone(input, &p) {
        report_project(&p, kits)?;
        return p.compile(kits).map_err(|findings| {
            let msgs: Vec<String> = findings.into_iter().map(|f| f.diagnostic.message).collect();
            miette!("cannot compile {}: {}", input.display(), msgs.join("; "))
        });
    }
    own_problems(input, &p)?;
    let (name, src) = (input.display().to_string(), &p.files[0].src);
    match p.compile(kits) {
        Ok(b) => {
            print_diagnostics(&name, src, p.locate(0, b.warnings.clone()));
            Ok(b)
        }
        Err(findings) => Err(failure(&name, src, p.locate(0, findings))),
    }
}

/// `out/rel`, refused if any part of it under `out` is a symlink: a link
/// in the bundle must not lead a write or a delete somewhere else. (`rel`
/// is a [`rhizo_compile::safe_path`], so it has no `..` either.)
fn contained(out: &Path, rel: &str) -> Result<PathBuf> {
    let mut p = out.to_path_buf();
    for seg in rel.split('/') {
        p.push(seg);
        if std::fs::symlink_metadata(&p).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(miette!("refusing to touch {}: {} is a symlink", out.join(rel).display(), p.display()));
        }
    }
    Ok(p)
}

fn compile(input: &Path, out: &Path, check_only: bool) -> Result<()> {
    let bundle = bundle_for(input, &Kits::builtin())?;

    // Files a previous compile generated, with the hash they had then.
    let old: Vec<(String, String)> = std::fs::read_to_string(out.join(rhizo_compile::MANIFEST))
        .map(|m| rhizo_compile::manifest_files(&m))
        .unwrap_or_default();
    let (mut added, mut changed, mut removed, mut kept) = (0, 0, 0, Vec::new());
    for (path, file) in &bundle.files {
        let target = out.join(path);
        match std::fs::read(&target) {
            Ok(bytes) if bytes == file.contents.as_bytes() => continue,
            Ok(_) => changed += 1,
            Err(_) => added += 1,
        }
        if check_only {
            eprintln!("out of date: {}", target.display());
            continue;
        }
        contained(out, path)?;
        if let Some(dir) = target.parent() {
            std::fs::create_dir_all(dir).map_err(|e| miette!("cannot create {}: {e}", dir.display()))?;
        }
        std::fs::write(&target, &file.contents).map_err(|e| miette!("cannot write {}: {e}", target.display()))?;
    }
    for (path, hash) in old.iter().filter(|(p, _)| !bundle.files.contains_key(p)) {
        let target = out.join(path);
        let Ok(bytes) = std::fs::read(&target) else { continue };
        // Someone edited a generated file: leave it and say so.
        if rhizo_compile::sha256(&bytes) != *hash {
            kept.push(target);
            continue;
        }
        removed += 1;
        if check_only {
            eprintln!("stale: {}", target.display());
        } else {
            contained(out, path)?;
            std::fs::remove_file(&target).map_err(|e| miette!("cannot remove {}: {e}", target.display()))?;
        }
    }
    for k in &kept {
        eprintln!("warning: {} is no longer generated but was edited by hand; left in place", k.display());
    }
    let summary = format!("{added} added, {changed} changed, {removed} removed");
    if check_only {
        if added + changed + removed > 0 {
            eprintln!("{} is out of date ({summary}); run `rhizo compile`", out.display());
            std::process::exit(1);
        }
        eprintln!("✓ {} is up to date", out.display());
    } else if added + changed + removed == 0 {
        eprintln!("✓ {} is up to date ({} files)", out.display(), bundle.files.len());
    } else {
        eprintln!("wrote {} ({summary})", out.display());
    }
    Ok(())
}

fn types(name: Option<&str>) -> Result<()> {
    let kits = Kits::builtin();
    if let Some(def) = name.and_then(|n| kits.find_type(n)) {
        let what = if def.frame { "frame type" } else { "node type" };
        println!("{} ({what}): {}", def.name, def.label);
        if let Some(d) = &def.description {
            println!("{d}");
        }
        if !def.parents.is_empty() {
            println!("\ninside: {}", def.parents.join(", "));
        }
        if let Some(s) = def.shape {
            println!("shape: {}", s.name());
        }
        if !def.ports.is_empty() {
            println!();
        }
        for p in &def.ports {
            let kind = p.port.kind.map(|k| format!(" kind={}", k.name())).unwrap_or_default();
            let default = if p.default { " default" } else { "" };
            println!("port {} {:?}{kind}{default}", p.port.dir.name(), p.port.name.as_str());
        }
        println!("\nprops schema:\n{}", serde_json::to_string_pretty(&def.schema).into_diagnostic()?);
        return Ok(());
    }
    let selected: Vec<_> = kits.iter().filter(|k| name.is_none_or(|n| n == k.name)).collect();
    if selected.is_empty() {
        return Err(miette!("no kit or type `{}` (see `rhizo types`)", name.unwrap_or_default()));
    }
    for kit in selected {
        println!("{} {}: {}", kit.name, kit.version, kit.description.as_deref().unwrap_or(""));
        for t in kit.types.values() {
            let what = if t.frame { "frame" } else { "node" };
            println!("  {:<15} {what:<6} {}", t.name, t.prop_names().join(" "));
        }
    }
    Ok(())
}

const MAX_SHOWN: usize = 12;

fn print_diagnostics(name: &str, src: &str, mut diags: Vec<Diagnostic>) {
    if diags.is_empty() {
        return;
    }
    let hidden = diags.len().saturating_sub(MAX_SHOWN);
    diags.truncate(MAX_SHOWN);
    eprintln!("{:?}", miette::Report::new(Report::new(name, src, diags)));
    if hidden > 0 {
        eprintln!("… and {hidden} more");
    }
}

fn produce(doc: &Document, to: Format, args: &RenderArgs) -> Result<Vec<u8>> {
    if let Some(warning) = rhizo_interop::dropped_features(doc, to) {
        eprintln!("warning: {warning}");
    }
    let theme = args
        .theme
        .as_deref()
        .map(|t| Theme::by_name(t).ok_or_else(|| miette!("unknown theme `{t}` (light, dark, paper)")))
        .transpose()?
        .unwrap_or_else(|| rhizo_render::theme_for(doc, None));
    if to.is_image() {
        let fonts = Fonts::system(theme.font_family);
        let diagram = rhizo_render::layout(doc, &theme, &fonts);
        let svg = rhizo_render::to_svg(&diagram, &SvgOptions { transparent: args.transparent, ..Default::default() });
        return match to {
            Format::Svg => Ok(svg.into_bytes()),
            Format::Png => rhizo_render::svg_to_png(&svg, &fonts, args.scale).map_err(|e| miette!("{e}")),
            _ => rhizo_render::svg_to_pdf(&svg, &fonts, doc.title.as_deref()).map_err(|e| miette!("{e}")),
        };
    }
    let needs_metrics = matches!(to, Format::JsonCanvas | Format::DrawIo | Format::GraphMl);
    let text = if needs_metrics {
        let fonts = Fonts::system(theme.font_family);
        rhizo_interop::export(doc, to, &theme, &fonts)
    } else {
        rhizo_interop::export(doc, to, &theme, &rhizo_layout::Heuristic)
    };
    text.map(String::into_bytes).map_err(|e| miette!("{e}"))
}

fn write_output(path: &Path, bytes: &[u8]) -> Result<()> {
    if path.as_os_str() == "-" {
        let mut out = std::io::stdout().lock();
        if out.is_terminal() && bytes.iter().take(512).any(|b| *b == 0) {
            return Err(miette!("refusing to write binary output to a terminal"));
        }
        return out.write_all(bytes).into_diagnostic();
    }
    std::fs::write(path, bytes).map_err(|e| miette!("cannot write {}: {e}", path.display()))
}

/// OpenTofu options with the providers the person allowed for this run, said aloud.
fn iac_options(allow_providers: Vec<String>) -> rhizo_iac::Options {
    for p in &allow_providers {
        eprintln!("allowing provider {p} for this run: OpenTofu installs and runs it");
    }
    rhizo_iac::Options { allow_providers, ..Default::default() }
}

fn report_written(path: &Path) {
    if path.as_os_str() != "-" {
        eprintln!("wrote {}", path.display());
    }
}

fn modified(path: &Path) -> Option<SystemTime> {
    std::fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn watch(input: &Path, output: Option<PathBuf>, args: &RenderArgs) -> Result<()> {
    let output = output.unwrap_or_else(|| input.with_extension("svg"));
    let to = resolve_format(&output, None)?;
    eprintln!("watching {} → {} (ctrl-c to stop)", input.display(), output.display());
    let mut last = None;
    loop {
        let now = modified(input);
        if now != last {
            last = now;
            let started = std::time::Instant::now();
            match load(input, None).and_then(|p| produce(&p.drawable(0, &Kits::builtin()), to, args)) {
                Ok(bytes) => {
                    write_output(&output, &bytes)?;
                    eprintln!("rendered in {:.0?}", started.elapsed());
                }
                Err(e) => eprintln!("{e:?}"),
            }
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}
