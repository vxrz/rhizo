use crate::Color;

/// Design tokens every renderer reads. Documents pick one by name; individual
/// nodes override it through [`crate::Style`].
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub name: &'static str,
    pub background: Color,
    /// CSS font-family list. The first installed family is used for measuring.
    pub font_family: &'static str,
    pub font_size: f32,
    pub font_weight: u16,
    pub root_font_size: f32,
    pub root_font_weight: u16,
    pub text: Color,
    pub muted_text: Color,
    pub node_fill: Color,
    pub node_stroke: Color,
    pub stroke_width: f32,
    pub corner_radius: f32,
    pub root_fill: Color,
    pub root_text: Color,
    pub edge: Color,
    pub edge_width: f32,
    pub frame_fill: Color,
    pub frame_stroke: Color,
    pub frame_label: Color,
    /// Branch colors for mindmaps and trees, in order.
    pub palette: &'static [Color],
}

pub const FONT_STACK: &str =
    "Inter, \"SF Pro Text\", -apple-system, \"Segoe UI\", \"Helvetica Neue\", Helvetica, Arial, sans-serif";

const LIGHT_PALETTE: &[Color] = &[
    Color::hex(0x6366f1),
    Color::hex(0x0ea5e9),
    Color::hex(0x10b981),
    Color::hex(0xf59e0b),
    Color::hex(0xef4444),
    Color::hex(0xec4899),
    Color::hex(0x8b5cf6),
    Color::hex(0x14b8a6),
];

const DARK_PALETTE: &[Color] = &[
    Color::hex(0x818cf8),
    Color::hex(0x38bdf8),
    Color::hex(0x34d399),
    Color::hex(0xfbbf24),
    Color::hex(0xf87171),
    Color::hex(0xf472b6),
    Color::hex(0xa78bfa),
    Color::hex(0x2dd4bf),
];

const PAPER_PALETTE: &[Color] = &[
    Color::hex(0xb4532a),
    Color::hex(0x3f6c8a),
    Color::hex(0x5a7d3a),
    Color::hex(0xa77b1c),
    Color::hex(0x8a3f6c),
    Color::hex(0x2f7a72),
];

impl Theme {
    pub const NAMES: [&'static str; 3] = ["light", "dark", "paper"];

    pub fn light() -> Theme {
        Theme {
            name: "light",
            background: Color::hex(0xffffff),
            font_family: FONT_STACK,
            font_size: 14.0,
            font_weight: 400,
            root_font_size: 17.0,
            root_font_weight: 600,
            text: Color::hex(0x18181b),
            muted_text: Color::hex(0x71717a),
            node_fill: Color::hex(0xffffff),
            node_stroke: Color::hex(0xd4d4d8),
            stroke_width: 1.25,
            corner_radius: 8.0,
            root_fill: Color::hex(0x18181b),
            root_text: Color::hex(0xfafafa),
            edge: Color::hex(0xa1a1aa),
            edge_width: 1.5,
            frame_fill: Color::hex(0xfafafa),
            frame_stroke: Color::hex(0xe4e4e7),
            frame_label: Color::hex(0x71717a),
            palette: LIGHT_PALETTE,
        }
    }

    pub fn dark() -> Theme {
        Theme {
            name: "dark",
            background: Color::hex(0x111113),
            text: Color::hex(0xf4f4f5),
            muted_text: Color::hex(0xa1a1aa),
            node_fill: Color::hex(0x1c1c20),
            node_stroke: Color::hex(0x3f3f46),
            root_fill: Color::hex(0xf4f4f5),
            root_text: Color::hex(0x111113),
            edge: Color::hex(0x71717a),
            frame_fill: Color::hex(0x18181b),
            frame_stroke: Color::hex(0x27272a),
            frame_label: Color::hex(0xa1a1aa),
            palette: DARK_PALETTE,
            ..Theme::light()
        }
    }

    /// Warm, low-contrast, print friendly.
    pub fn paper() -> Theme {
        Theme {
            name: "paper",
            background: Color::hex(0xfbf8f1),
            font_family: "Charter, \"Iowan Old Style\", Georgia, \"Times New Roman\", serif",
            font_size: 15.0,
            text: Color::hex(0x2b2620),
            muted_text: Color::hex(0x7a7064),
            node_fill: Color::hex(0xfffdf8),
            node_stroke: Color::hex(0xd9cfbf),
            root_fill: Color::hex(0x2b2620),
            root_text: Color::hex(0xfbf8f1),
            edge: Color::hex(0xb3a794),
            frame_fill: Color::hex(0xf5f0e4),
            frame_stroke: Color::hex(0xe3d9c6),
            frame_label: Color::hex(0x7a7064),
            corner_radius: 4.0,
            palette: PAPER_PALETTE,
            ..Theme::light()
        }
    }

    pub fn by_name(name: &str) -> Option<Theme> {
        match name {
            "light" | "default" => Some(Theme::light()),
            "dark" => Some(Theme::dark()),
            "paper" => Some(Theme::paper()),
            _ => None,
        }
    }

    /// Branch color for the `i`-th child of a root.
    pub fn branch(&self, i: usize) -> Color {
        self.palette[i % self.palette.len()]
    }

    /// `c` washed toward the background. `amount = 1` is the background itself.
    pub fn tint(&self, c: Color, amount: f32) -> Color {
        c.mix(self.background, amount)
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::light()
    }
}
