use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use rhizo_layout::TextMeasure;
use usvg::fontdb::{self, Database, Family, Query, Weight};

/// System fonts, shared between text measurement and rasterization so PNG
/// and PDF output match the sizes layout computed.
pub struct Fonts {
    db: Arc<Database>,
    families: Vec<String>,
    cache: Mutex<HashMap<u16, Face>>,
}

struct Face {
    id: Option<fontdb::ID>,
    units_per_em: f32,
    /// Glyph id and advance per character; `None` when the font lacks it.
    glyphs: HashMap<char, Option<(u16, u16)>>,
    /// The font file, loaded on first [`Fonts::shape`].
    data: Option<(Arc<Vec<u8>>, u32)>,
}

/// A line of text as positioned glyphs, measured exactly as layout measured it.
pub struct Shaped {
    /// The font file and its index in a collection.
    pub font: (Arc<Vec<u8>>, u32),
    /// Glyph id and x offset from the line start, in pixels.
    pub glyphs: Vec<(u16, f32)>,
    pub width: f32,
}

impl Fonts {
    /// Loads every installed font. Takes a few tens of milliseconds.
    pub fn system(font_family: &str) -> Fonts {
        let mut db = Database::new();
        db.load_system_fonts();
        Fonts::with_database(db, font_family)
    }

    pub fn with_database(mut db: Database, font_family: &str) -> Fonts {
        db.set_sans_serif_family(
            first_installed(&db, &["Inter", "Helvetica Neue", "Helvetica", "Arial", "DejaVu Sans", "Liberation Sans"])
                .unwrap_or("Arial"),
        );
        db.set_serif_family(
            first_installed(&db, &["Charter", "Georgia", "Times New Roman", "DejaVu Serif", "Liberation Serif"])
                .unwrap_or("Times New Roman"),
        );
        Fonts { db: Arc::new(db), families: parse_family_list(font_family), cache: Mutex::new(HashMap::new()) }
    }

    pub fn database(&self) -> Arc<Database> {
        self.db.clone()
    }

    /// The family layout actually measures with: the first installed one.
    pub fn resolved_family(&self) -> Option<String> {
        let id = self.query(400)?;
        self.db.face(id).and_then(|f| f.families.first().map(|(name, _)| name.clone()))
    }

    fn query(&self, weight: u16) -> Option<fontdb::ID> {
        let families: Vec<Family> = self
            .families
            .iter()
            .map(|f| match f.as_str() {
                "sans-serif" => Family::SansSerif,
                "serif" => Family::Serif,
                "monospace" => Family::Monospace,
                name => Family::Name(name),
            })
            .chain([Family::SansSerif])
            .collect();
        self.db.query(&Query { families: &families, weight: Weight(weight), ..Query::default() })
    }
}

fn first_installed<'a>(db: &Database, names: &[&'a str]) -> Option<&'a str> {
    names.iter().copied().find(|n| db.faces().any(|f| f.families.iter().any(|(fam, _)| fam == n)))
}

fn parse_family_list(css: &str) -> Vec<String> {
    css.split(',')
        .map(|f| f.trim().trim_matches(['"', '\'']).to_string())
        .filter(|f| !f.is_empty() && !matches!(f.as_str(), "-apple-system" | "system-ui" | "BlinkMacSystemFont"))
        .collect()
}

/// Per character: glyph id and advance in font units, or `None`.
type Run = (f32, Vec<Option<(u16, u16)>>, Option<(Arc<Vec<u8>>, u32)>);

impl Fonts {
    fn run(&self, text: &str, font_weight: u16, with_data: bool) -> Option<Run> {
        let bucket = (font_weight / 100).clamp(1, 9) * 100;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let face = cache.entry(bucket).or_insert_with(|| Face {
            id: self.query(bucket),
            units_per_em: 1000.0,
            glyphs: HashMap::new(),
            data: None,
        });
        let id = face.id?;
        if text.chars().any(|c| !face.glyphs.contains_key(&c)) {
            let missing: Vec<char> = text.chars().filter(|c| !face.glyphs.contains_key(c)).collect();
            let found = self.db.with_face_data(id, |data, index| {
                let parsed = ttf_parser::Face::parse(data, index).ok()?;
                let upem = parsed.units_per_em() as f32;
                let glyphs: Vec<(char, Option<(u16, u16)>)> = missing
                    .iter()
                    .map(|&c| {
                        let g = parsed.glyph_index(c);
                        (c, g.and_then(|g| Some((g.0, parsed.glyph_hor_advance(g)?))))
                    })
                    .collect();
                Some((upem, glyphs))
            });
            if let Some(Some((upem, glyphs))) = found {
                face.units_per_em = upem;
                face.glyphs.extend(glyphs);
            } else {
                face.glyphs.extend(missing.into_iter().map(|c| (c, None)));
            }
        }
        if with_data && face.data.is_none() {
            face.data = self.db.with_face_data(id, |data, index| (Arc::new(data.to_vec()), index));
        }
        let glyphs = text.chars().map(|c| face.glyphs.get(&c).copied().flatten()).collect();
        Some((face.units_per_em, glyphs, face.data.clone()))
    }

    /// Positions the glyphs of one line with the same advances [`TextMeasure`]
    /// reports, so drawn text fits the boxes layout made for it.
    pub fn shape(&self, text: &str, font_size: f32, font_weight: u16) -> Option<Shaped> {
        let (upem, run, font) = self.run(text, font_weight, true)?;
        let mut x = 0.0;
        let mut glyphs = Vec::with_capacity(run.len());
        for (c, g) in text.chars().zip(run) {
            // shortcut: characters missing from the font draw as .notdef (no
            // fallback font, no kerning); fine for Latin labels, upgrade with parley.
            let (id, advance) = match g {
                Some((id, adv)) => (id, adv as f32 / upem * font_size),
                None => (0, rhizo_layout::Heuristic.width(&c.to_string(), font_size, font_weight)),
            };
            glyphs.push((id, x));
            x += advance;
        }
        Some(Shaped { font: font?, glyphs, width: x })
    }
}

impl TextMeasure for Fonts {
    fn width(&self, text: &str, font_size: f32, font_weight: u16) -> f32 {
        let Some((upem, run, _)) = self.run(text, font_weight, false) else {
            return rhizo_layout::Heuristic.width(text, font_size, font_weight);
        };
        text.chars()
            .zip(run)
            .map(|(c, g)| match g {
                Some((_, adv)) => adv as f32 / upem * font_size,
                // Not in this font; the renderer will fall back. Guess.
                None => rhizo_layout::Heuristic.width(&c.to_string(), font_size, font_weight),
            })
            .sum()
    }
}
