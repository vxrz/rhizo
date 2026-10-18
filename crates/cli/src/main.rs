//! `rhizo`: render, convert, check and format diagrams.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use clap::{Parser, Subcommand};
use miette::{IntoDiagnostic, Result, miette};
use rhizo_core::{Document, Theme};
use rhizo_interop::Format;
use rhizo_lang::{Diagnostic, Report};
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
    /// Report problems without writing anything.
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
    /// Print a starter document.
    New {
        /// mindmap, flow, or canvas.
        #[arg(default_value = "mindmap")]
        kind: String,
    },
    /// Print the JSON Schema for the JSON/YAML forms.
    Schema,
    /// List supported formats.
    Formats,
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Render { input, output, format, render } => {
            let output = output.unwrap_or_else(|| input.with_extension(format.as_deref().unwrap_or("svg")));
            let doc = load(&input, None)?;
            let to = resolve_format(&output, format.as_deref())?;
            if !to.is_image() {
                return Err(miette!("`render` writes svg, png or pdf; use `rhizo convert` for {}", to.name()));
            }
            write_output(&output, &produce(&doc, to, &render)?)?;
            report_written(&output);
        }
        Command::Convert { input, output, from, to, render } => {
            let doc = load(&input, from.as_deref())?;
            let to = resolve_format(&output, to.as_deref())?;
            write_output(&output, &produce(&doc, to, &render)?)?;
            report_written(&output);
        }
        Command::Check { inputs } => {
            let mut failed = 0;
            for input in &inputs {
                match load(input, None) {
                    Ok(doc) => {
                        eprintln!("✓ {} ({} nodes, {} edges)", input.display(), doc.nodes.len(), doc.edges.len())
                    }
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
                let src = read_input(input)?;
                let doc = load_src(input, &src, Format::Rhizo)?;
                let formatted = rhizo_lang::to_kdl(&doc);
                if formatted == src {
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
        Command::New { kind } => print!("{}", starter(&kind)?),
        Command::Schema => {
            println!("{}", serde_json::to_string_pretty(&rhizo_lang::json_schema()).into_diagnostic()?)
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

fn load(path: &Path, from: Option<&str>) -> Result<Document> {
    let format = input_format(path, from)?;
    let src = read_input(path)?;
    load_src(path, &src, format)
}

fn load_src(path: &Path, src: &str, format: Format) -> Result<Document> {
    let name = path.display().to_string();
    match rhizo_interop::import(src, format) {
        Ok((doc, warnings)) => {
            print_diagnostics(&name, src, warnings);
            Ok(doc)
        }
        Err(mut errors) => {
            errors.sort_by_key(|d| !d.is_error());
            let hidden = errors.len().saturating_sub(MAX_SHOWN);
            errors.truncate(MAX_SHOWN);
            if hidden > 0 {
                eprintln!("(showing {MAX_SHOWN} of {} problems)", MAX_SHOWN + hidden);
            }
            Err(Report::new(&name, src, errors).into())
        }
    }
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
            match load(input, None).and_then(|doc| produce(&doc, to, args)) {
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

fn starter(kind: &str) -> Result<&'static str> {
    Ok(match kind {
        "mindmap" | "map" => {
            r##"rhizo version=1
title "My idea"

// `-` adds a node; nesting makes children. Ids come from labels.
- "My idea" {
    - "Why" {
        - "Problem"
        - "Who has it"
    }
    - "How" {
        - "First step"
        - "Risks" fill="#fee2e2"
    }
    - "Next"
}
"##
        }
        "flow" | "flowchart" => {
            r##"rhizo version=1
title "Process"
layout "layered"
direction "down"

node "Start" shape=pill
node "Ready?" id=ready shape=diamond
node "Do the thing"
node "Done" shape=pill

// Refer to nodes by id or by label. Unknown names create new nodes.
edge start ready
edge ready "Do the thing" label="yes"
edge ready start label="no" line=dashed
edge "Do the thing" done
"##
        }
        "canvas" | "free" => {
            r##"rhizo version=1
title "Board"
layout "free"

node "Idea" x=0 y=0
node "Related" x=260 y=40
node "Question?" x=120 y=180 shape=ellipse
edge idea related
edge idea "Question?" line=dotted arrow=none
"##
        }
        other => return Err(miette!("unknown starter `{other}`: try mindmap, flow or canvas")),
    })
}
