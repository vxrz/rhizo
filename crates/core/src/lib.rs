//! The rhizo document model.
//!
//! Every input format (the KDL text format, JSON, YAML, Mermaid, OPML, the
//! editor) is lowered into a [`Document`], and every output (SVG, PDF, PNG,
//! Mermaid, D2, ...) is produced from one. Nothing in this crate depends on a
//! UI, a font stack or a file system, so it builds for every target,
//! including `wasm32-unknown-unknown`.

pub mod color;
pub mod geom;
pub mod model;
pub mod style;
pub mod theme;

pub use color::Color;
pub use geom::{Point, Rect, Size};
pub use model::*;
pub use style::{Arrow, LineStyle, Shape, Style};
pub use theme::Theme;

/// Turns arbitrary text into an identifier: lowercase ASCII letters, digits and
/// dashes. Used to derive node ids from labels.
pub fn slugify(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut dash = false;
    for c in text.chars() {
        if c.is_alphanumeric() {
            for l in c.to_lowercase() {
                if l.is_ascii_alphanumeric() {
                    out.push(l);
                    dash = false;
                } else if let Some(a) = ascii_fold(l) {
                    out.push(a);
                    dash = false;
                }
            }
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.is_empty() {
        out.push_str("node");
    }
    out
}

fn ascii_fold(c: char) -> Option<char> {
    Some(match c {
        'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' => 'a',
        'ç' => 'c',
        'è' | 'é' | 'ê' | 'ë' => 'e',
        'ì' | 'í' | 'î' | 'ï' => 'i',
        'ñ' => 'n',
        'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' => 'o',
        'ù' | 'ú' | 'û' | 'ü' => 'u',
        'ý' | 'ÿ' => 'y',
        'ß' => 's',
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::slugify;

    #[test]
    fn slugs() {
        assert_eq!(slugify("Hello, World!"), "hello-world");
        assert_eq!(slugify("  Über Größe "), "uber-grose");
        assert_eq!(slugify("???"), "node");
        assert_eq!(slugify("API v2.0"), "api-v2-0");
    }
}
