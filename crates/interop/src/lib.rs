//! Conversion between rhizo documents and other formats.
//!
//! | Format      | Import | Export | Notes                                                    |
//! |-------------|:------:|:------:|----------------------------------------------------------|
//! | rhizo (KDL) |   ✓    |   ✓    | native, lossless                                         |
//! | JSON / YAML |   ✓    |   ✓    | same schema as KDL, lossless                             |
//! | Mermaid     |   ✓    |   ✓    | flowchart, mindmap, sequenceDiagram; stateDiagram import |
//! | D2          |        |   ✓    | sequence frames as `sequence_diagram`                    |
//! | Graphviz    |        |   ✓    | DOT                                                      |
//! | OPML        |   ✓    |   ✓    | outline only                                             |
//! | FreeMind    |   ✓    |   ✓    | `.mm`, also read by Freeplane and XMind                  |
//! | Markdown    |   ✓    |   ✓    | headings and lists (markmap style)                       |
//! | JSON Canvas |   ✓    |   ✓    | Obsidian canvas, with positions                          |
//! | draw.io     |        |   ✓    | diagrams.net, with positions                             |
//! | GraphML     |        |   ✓    | yEd, Gephi, Cytoscape, networkx                          |
//!
//! Nested frames export as nested subgraphs (Mermaid, DOT), containers (D2,
//! draw.io) and groups inside groups (JSON Canvas); the other formats list
//! them flat, see [`keeps_nested_frames`].

pub mod canvas;
pub mod graphviz;
pub mod mermaid;
pub mod outline;
mod util;

use rhizo_core::{Document, Theme};
use rhizo_lang::{Diagnostic, Syntax, lower};
use rhizo_layout::{TextMeasure, layout};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Format {
    Rhizo,
    Json,
    Yaml,
    Mermaid,
    D2,
    Dot,
    Opml,
    FreeMind,
    Markdown,
    JsonCanvas,
    DrawIo,
    GraphMl,
    Svg,
    Png,
    Pdf,
}

impl Format {
    pub const ALL: [Format; 15] = [
        Format::Rhizo,
        Format::Json,
        Format::Yaml,
        Format::Mermaid,
        Format::D2,
        Format::Dot,
        Format::Opml,
        Format::FreeMind,
        Format::Markdown,
        Format::JsonCanvas,
        Format::DrawIo,
        Format::GraphMl,
        Format::Svg,
        Format::Png,
        Format::Pdf,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Format::Rhizo => "rhizo",
            Format::Json => "json",
            Format::Yaml => "yaml",
            Format::Mermaid => "mermaid",
            Format::D2 => "d2",
            Format::Dot => "dot",
            Format::Opml => "opml",
            Format::FreeMind => "freemind",
            Format::Markdown => "markdown",
            Format::JsonCanvas => "canvas",
            Format::DrawIo => "drawio",
            Format::GraphMl => "graphml",
            Format::Svg => "svg",
            Format::Png => "png",
            Format::Pdf => "pdf",
        }
    }

    pub fn extensions(self) -> &'static [&'static str] {
        match self {
            Format::Rhizo => &["rhizo", "kdl"],
            Format::Json => &["json"],
            Format::Yaml => &["yaml", "yml"],
            Format::Mermaid => &["mmd", "mermaid"],
            Format::D2 => &["d2"],
            Format::Dot => &["dot", "gv"],
            Format::Opml => &["opml"],
            Format::FreeMind => &["mm"],
            Format::Markdown => &["md", "markdown"],
            Format::JsonCanvas => &["canvas"],
            Format::DrawIo => &["drawio"],
            Format::GraphMl => &["graphml"],
            Format::Svg => &["svg"],
            Format::Png => &["png"],
            Format::Pdf => &["pdf"],
        }
    }

    pub fn from_extension(ext: &str) -> Option<Format> {
        let ext = ext.to_ascii_lowercase();
        Format::ALL.into_iter().find(|f| f.extensions().contains(&ext.as_str()))
    }

    pub fn from_path(path: &std::path::Path) -> Option<Format> {
        Format::from_extension(path.extension()?.to_str()?)
    }

    pub fn from_name(name: &str) -> Option<Format> {
        let name = name.to_ascii_lowercase();
        Format::ALL.into_iter().find(|f| f.name() == name).or_else(|| Format::from_extension(&name)).or(
            match name.as_str() {
                "kdl" => Some(Format::Rhizo),
                "graphviz" => Some(Format::Dot),
                "jsoncanvas" | "obsidian" => Some(Format::JsonCanvas),
                "mindnode" | "freeplane" | "xmind" => Some(Format::FreeMind),
                _ => None,
            },
        )
    }

    pub fn can_import(self) -> bool {
        matches!(
            self,
            Format::Rhizo
                | Format::Json
                | Format::Yaml
                | Format::Mermaid
                | Format::Opml
                | Format::FreeMind
                | Format::Markdown
                | Format::JsonCanvas
        )
    }

    /// The rhizo syntax this format is written in: KDL, JSON or YAML.
    pub fn syntax(self) -> Option<Syntax> {
        match self {
            Format::Rhizo => Some(Syntax::Kdl),
            Format::Json => Some(Syntax::Json),
            Format::Yaml => Some(Syntax::Yaml),
            _ => None,
        }
    }

    /// Images are rendered by `rhizo-render`, not here.
    pub fn is_image(self) -> bool {
        matches!(self, Format::Svg | Format::Png | Format::Pdf)
    }

    pub fn is_binary(self) -> bool {
        matches!(self, Format::Png | Format::Pdf)
    }
}

/// Formats that keep types, props, ports and provenance. Everything else
/// drops them; see [`dropped_features`].
pub fn keeps_typed_features(format: Format) -> bool {
    matches!(format, Format::Rhizo | Format::Json | Format::Yaml | Format::GraphMl | Format::DrawIo)
}

/// Formats that draw frames inside frames. The outline formats and GraphML
/// list every frame at the top level instead (no node is lost).
pub fn keeps_nested_frames(format: Format) -> bool {
    !matches!(format, Format::Opml | Format::FreeMind | Format::Markdown | Format::GraphMl)
}

/// One warning per conversion (not per node) when typed data or frame
/// nesting will be lost.
pub fn dropped_features(doc: &Document, format: Format) -> Option<String> {
    let typed = (doc.is_typed() && !keeps_typed_features(format) && !format.is_image())
        .then(|| format!("{} has no place for types, props, ports or provenance; they were left out", format.name()));
    let nested = (doc.frames.iter().any(|f| f.parent.is_some()) && !keeps_nested_frames(format))
        .then(|| format!("{} cannot nest frames; nested frames are listed at the top level", format.name()));
    match (typed, nested) {
        (Some(a), Some(b)) => Some(format!("{a}; {b}")),
        (a, b) => a.or(b),
    }
}

/// Reads `src` as `format`. Warnings are returned alongside the document;
/// any error fails the import.
pub fn import(src: &str, format: Format) -> Result<(Document, Vec<Diagnostic>), Vec<Diagnostic>> {
    let mut warnings = Vec::new();
    if let Some(syntax) = format.syntax() {
        return rhizo_lang::load(src, syntax).into_result();
    }
    let file = match format {
        Format::Mermaid => {
            let (file, w) = mermaid::import(src)?;
            warnings = w;
            file
        }
        Format::Opml => outline::opml_import(src)?,
        Format::FreeMind => outline::freemind_import(src)?,
        Format::Markdown => outline::markdown_import(src)?,
        Format::JsonCanvas => canvas::canvas_import(src)?,
        other => {
            return Err(vec![Diagnostic::error(format!("rhizo cannot read {} files", other.name()), None)]);
        }
    };
    let (doc, mut diags) = lower(&file);
    diags.extend(warnings);
    if diags.iter().any(Diagnostic::is_error) { Err(diags) } else { Ok((doc, diags)) }
}

/// Writes `doc` as a text format. Formats with geometry are laid out with
/// `theme` and `measure` first.
pub fn export(doc: &Document, format: Format, theme: &Theme, measure: &dyn TextMeasure) -> Result<String, String> {
    Ok(match format {
        Format::Rhizo => rhizo_lang::to_kdl(doc),
        Format::Json => rhizo_lang::to_json(doc),
        Format::Yaml => rhizo_lang::to_yaml(doc),
        Format::Mermaid => mermaid::export(doc),
        Format::D2 => graphviz::d2(doc),
        Format::Dot => graphviz::dot(doc),
        Format::Opml => outline::opml_export(doc),
        Format::FreeMind => outline::freemind_export(doc),
        Format::Markdown => outline::markdown_export(doc),
        Format::JsonCanvas => canvas::canvas_export(doc, &layout(doc, theme, measure)),
        Format::DrawIo => canvas::drawio_export(doc, &layout(doc, theme, measure)),
        Format::GraphMl => canvas::graphml_export(doc, &layout(doc, theme, measure)),
        Format::Svg | Format::Png | Format::Pdf => {
            return Err(format!("{} is an image format; render it with rhizo-render", format.name()));
        }
    })
}

#[cfg(test)]
mod tests;
