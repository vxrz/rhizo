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
    advances: HashMap<char, Option<u16>>,
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

impl TextMeasure for Fonts {
    fn width(&self, text: &str, font_size: f32, font_weight: u16) -> f32 {
        let bucket = (font_weight / 100).clamp(1, 9) * 100;
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let face = cache.entry(bucket).or_insert_with(|| Face {
            id: self.query(bucket),
            units_per_em: 1000.0,
            advances: HashMap::new(),
        });
        let Some(id) = face.id else {
            return rhizo_layout::Heuristic.width(text, font_size, font_weight);
        };
        if text.chars().any(|c| !face.advances.contains_key(&c)) {
            let missing: Vec<char> = text.chars().filter(|c| !face.advances.contains_key(c)).collect();
            let found = self.db.with_face_data(id, |data, index| {
                let parsed = ttf_parser::Face::parse(data, index).ok()?;
                let upem = parsed.units_per_em() as f32;
                let adv: Vec<(char, Option<u16>)> = missing
                    .iter()
                    .map(|&c| (c, parsed.glyph_index(c).and_then(|g| parsed.glyph_hor_advance(g))))
                    .collect();
                Some((upem, adv))
            });
            if let Some(Some((upem, adv))) = found {
                face.units_per_em = upem;
                face.advances.extend(adv);
            } else {
                face.advances.extend(missing.into_iter().map(|c| (c, None)));
            }
        }
        let upem = face.units_per_em;
        text.chars()
            .map(|c| match face.advances.get(&c).copied().flatten() {
                Some(a) => a as f32 / upem * font_size,
                // Not in this font; the renderer will fall back. Guess.
                None => rhizo_layout::Heuristic.width(&c.to_string(), font_size, font_weight),
            })
            .sum()
    }
}
