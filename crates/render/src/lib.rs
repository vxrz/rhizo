//! Rendering: SVG (written directly), PNG (resvg) and PDF (krilla).
//!
//! PNG and PDF are produced from the same SVG, so all three outputs agree.
//! PDFs keep text as real, selectable text with embedded font subsets.

pub mod fonts;
pub mod svg;

pub use fonts::{Fonts, Shaped};
pub use svg::{SvgOptions, to_svg};

use krilla::metadata::Metadata;
use krilla::page::PageSettings;
use krilla_svg::{SurfaceExt, SvgSettings};
use rhizo_core::{Document, Theme};
use rhizo_layout::Diagram;

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("could not read the generated SVG: {0}")]
    Svg(#[from] usvg::Error),
    #[error("the diagram is empty or too large to rasterize at this scale")]
    Size,
    #[error("PDF export failed: {0}")]
    Pdf(String),
}

/// The theme a document asks for, defaulting to light.
pub fn theme_for(doc: &Document, overrides: Option<&str>) -> Theme {
    overrides.or(doc.theme.as_deref()).and_then(Theme::by_name).unwrap_or_default()
}

/// Lays out with real font metrics.
pub fn layout(doc: &Document, theme: &Theme, fonts: &Fonts) -> Diagram {
    rhizo_layout::layout(doc, theme, fonts)
}

fn tree(svg: &str, fonts: &Fonts) -> Result<usvg::Tree, RenderError> {
    let opt = usvg::Options { fontdb: fonts.database(), ..usvg::Options::default() };
    Ok(usvg::Tree::from_str(svg, &opt)?)
}

/// Rasterizes at `scale` (2.0 for retina-sharp output).
pub fn svg_to_png(svg: &str, fonts: &Fonts, scale: f32) -> Result<Vec<u8>, RenderError> {
    let tree = tree(svg, fonts)?;
    let size = tree.size();
    let (w, h) = ((size.width() * scale).ceil() as u32, (size.height() * scale).ceil() as u32);
    if w == 0 || h == 0 || (w as u64) * (h as u64) > 400_000_000 {
        return Err(RenderError::Size);
    }
    let mut pixmap = resvg::tiny_skia::Pixmap::new(w, h).ok_or(RenderError::Size)?;
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|_| RenderError::Size)
}

/// One-page vector PDF sized to the diagram (1 SVG px = 0.75 pt, so the page
/// prints at the size it appears on screen).
pub fn svg_to_pdf(svg: &str, fonts: &Fonts, title: Option<&str>) -> Result<Vec<u8>, RenderError> {
    let tree = tree(svg, fonts)?;
    let (w, h) = (tree.size().width() * 0.75, tree.size().height() * 0.75);
    let mut doc = krilla::Document::new();
    let mut meta = Metadata::new().creator("rhizo".into()).producer(format!("rhizo {}", env!("CARGO_PKG_VERSION")));
    if let Some(t) = title {
        meta = meta.title(t.into());
    }
    doc.set_metadata(meta);
    let settings = PageSettings::from_wh(w, h).ok_or(RenderError::Size)?;
    let mut page = doc.start_page_with(settings);
    let mut surface = page.surface();
    let size = krilla::geom::Size::from_wh(w, h).ok_or(RenderError::Size)?;
    surface.draw_svg(&tree, size, SvgSettings::default()).ok_or(RenderError::Pdf("could not draw SVG".into()))?;
    surface.finish();
    page.finish();
    doc.finish().map_err(|e| RenderError::Pdf(format!("{e:?}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhizo_lang::{Syntax, load};

    const SRC: &str = r##"
title "Render test"
style "warn" fill="#fef3c7" stroke="#f59e0b"
- "Root" {
    - "A & B <tags>" class=warn note="hover me" link="https://example.com"
    - "Second branch with a fairly long label that has to wrap"
}
frame "Flow" layout=layered {
    node "Start" shape=pill
    node "Decide" shape=diamond
    node "Hex" shape=hexagon
    node "Round" shape=circle
    edge start decide hex label="go"
    edge decide round arrow=both line=dotted
}
"##;

    fn fonts() -> Fonts {
        Fonts::system(rhizo_core::theme::FONT_STACK)
    }

    fn svg(fonts: &Fonts) -> String {
        let doc = load(SRC, Syntax::Kdl).into_result().unwrap().0;
        let theme = theme_for(&doc, None);
        to_svg(&layout(&doc, &theme, fonts), &SvgOptions::default())
    }

    #[test]
    fn svg_is_wellformed_and_semantic() {
        let fonts = fonts();
        let s = svg(&fonts);
        assert!(s.starts_with("<svg"));
        assert!(s.contains(r#"<title id="rz-title">Render test</title>"#));
        assert!(s.contains("A &amp; B &lt;tags&gt;"));
        assert!(s.contains(r#"id="node-decide""#));
        assert!(s.contains("rz-shape-diamond"));
        assert!(s.contains("marker-start="));
        assert!(s.contains("<a href=\"https://example.com\""));
        // usvg is a strict parser: if it accepts the output, so will browsers.
        tree(&s, &fonts).unwrap();
    }

    #[test]
    fn png_and_pdf() {
        let fonts = fonts();
        let s = svg(&fonts);
        let png = svg_to_png(&s, &fonts, 2.0).unwrap();
        assert_eq!(&png[1..4], b"PNG");
        let pdf = svg_to_pdf(&s, &fonts, Some("Render test")).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.len() > 1000);
    }

    #[test]
    fn real_metrics_are_plausible() {
        use rhizo_layout::TextMeasure;
        let fonts = fonts();
        let w = fonts.width("Hello world", 14.0, 400);
        assert!((50.0..110.0).contains(&w), "width {w}");
        assert!(fonts.width("Hello world", 14.0, 700) >= w);
    }
}
