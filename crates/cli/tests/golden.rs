//! Golden files: every plain document renders and exports to exactly the
//! bytes recorded in `tests/golden/`. Guards the promise that new model
//! features never change plain diagrams (design doc profiles §2.10).
//!
//! Regenerate deliberately with `UPDATE_GOLDEN=1 cargo test -p rhizo-cli --test golden`.

use std::path::{Path, PathBuf};

use rhizo_core::Theme;
use rhizo_interop::Format;
use rhizo_layout::Heuristic;

const TEXT_FORMATS: [Format; 12] = [
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
];

fn inputs() -> Vec<PathBuf> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> = ["../../examples", "tests/fixtures"]
        .iter()
        .flat_map(|d| std::fs::read_dir(root.join(d)).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "rhizo"))
        .collect();
    files.sort();
    files
}

#[test]
fn plain_documents_are_byte_identical() {
    let update = std::env::var_os("UPDATE_GOLDEN").is_some();
    let golden = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/golden");
    let mut failures = Vec::new();
    for input in inputs() {
        let src = std::fs::read_to_string(&input).unwrap();
        let (doc, _) = rhizo_interop::import(&src, Format::Rhizo).expect("fixture parses");
        let theme = doc.theme.as_deref().and_then(Theme::by_name).unwrap_or_default();
        let stem = input.file_stem().unwrap().to_string_lossy().to_string();
        let mut outputs: Vec<(String, String)> = TEXT_FORMATS
            .iter()
            .map(|&f| {
                let text = rhizo_interop::export(&doc, f, &theme, &Heuristic).unwrap();
                (format!("{stem}.{}", f.extensions()[0]), text)
            })
            .collect();
        let diagram = rhizo_layout::layout(&doc, &theme, &Heuristic);
        outputs.push((format!("{stem}.svg"), rhizo_render::to_svg(&diagram, &Default::default())));
        for (name, text) in outputs {
            let path = golden.join(&name);
            if update {
                std::fs::write(&path, &text).unwrap();
            } else if std::fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
                failures.push(name);
            }
        }
    }
    assert!(failures.is_empty(), "output changed for: {failures:?} (UPDATE_GOLDEN=1 only if intended)");
}
