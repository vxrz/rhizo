use std::fmt;
use std::str::FromStr;

/// An sRGB color with alpha. Parsed from `#rgb`, `#rgba`, `#rrggbb`,
/// `#rrggbbaa` or a CSS named color; always written as lowercase hex.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    pub const fn hex(v: u32) -> Self {
        Self::rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    /// Linear blend: `t = 0` is `self`, `t = 1` is `other`.
    pub fn mix(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color { r: m(self.r, other.r), g: m(self.g, other.g), b: m(self.b, other.b), a: m(self.a, other.a) }
    }

    /// Relative luminance per WCAG 2.x.
    pub fn luminance(self) -> f32 {
        let ch = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.03928 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * ch(self.r) + 0.7152 * ch(self.g) + 0.0722 * ch(self.b)
    }

    pub fn contrast(self, other: Color) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }

    /// Whichever of `light` and `dark` reads better on top of `self`.
    pub fn readable_on(self, light: Color, dark: Color) -> Color {
        if self.contrast(light) >= self.contrast(dark) { light } else { dark }
    }

    pub fn is_opaque(self) -> bool {
        self.a == 255
    }

    pub fn opacity(self) -> f32 {
        self.a as f32 / 255.0
    }

    /// `#rrggbb`, dropping alpha. Pair with [`Color::opacity`] where alpha matters.
    pub fn to_hex_rgb(self) -> String {
        format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.a == 255 {
            write!(f, "#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            write!(f, "#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("invalid color `{0}`: expected #rgb, #rrggbb, #rrggbbaa or a CSS color name")]
pub struct ColorError(pub String);

impl FromStr for Color {
    type Err = ColorError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let t = s.trim();
        let err = || ColorError(s.to_string());
        if let Some(hex) = t.strip_prefix('#') {
            let nib = |c: u8| (c as char).to_digit(16).map(|d| d as u8);
            let bytes = hex.as_bytes();
            let digits: Option<Vec<u8>> = bytes.iter().map(|&c| nib(c)).collect();
            let d = digits.ok_or_else(err)?;
            return match d.len() {
                3 | 4 => Ok(Color { r: d[0] * 17, g: d[1] * 17, b: d[2] * 17, a: d.get(3).map_or(255, |a| a * 17) }),
                6 | 8 => Ok(Color {
                    r: d[0] << 4 | d[1],
                    g: d[2] << 4 | d[3],
                    b: d[4] << 4 | d[5],
                    a: if d.len() == 8 { d[6] << 4 | d[7] } else { 255 },
                }),
                _ => Err(err()),
            };
        }
        let lower = t.to_ascii_lowercase();
        NAMED
            .iter()
            .find(|(n, _)| *n == lower)
            .map(|(_, v)| Color::hex(*v))
            .or_else(|| (lower == "transparent").then_some(Color { r: 0, g: 0, b: 0, a: 0 }))
            .ok_or_else(err)
    }
}

/// The CSS named colors people actually type. The full list of 148 is not
/// worth the binary size; hex covers everything else.
const NAMED: &[(&str, u32)] = &[
    ("black", 0x000000),
    ("white", 0xffffff),
    ("gray", 0x808080),
    ("grey", 0x808080),
    ("silver", 0xc0c0c0),
    ("red", 0xff0000),
    ("maroon", 0x800000),
    ("orange", 0xffa500),
    ("gold", 0xffd700),
    ("yellow", 0xffff00),
    ("olive", 0x808000),
    ("lime", 0x00ff00),
    ("green", 0x008000),
    ("teal", 0x008080),
    ("cyan", 0x00ffff),
    ("aqua", 0x00ffff),
    ("blue", 0x0000ff),
    ("navy", 0x000080),
    ("indigo", 0x4b0082),
    ("purple", 0x800080),
    ("violet", 0xee82ee),
    ("magenta", 0xff00ff),
    ("fuchsia", 0xff00ff),
    ("pink", 0xffc0cb),
    ("brown", 0xa52a2a),
    ("beige", 0xf5f5dc),
    ("coral", 0xff7f50),
    ("crimson", 0xdc143c),
    ("salmon", 0xfa8072),
    ("tomato", 0xff6347),
    ("khaki", 0xf0e68c),
    ("lavender", 0xe6e6fa),
    ("plum", 0xdda0dd),
    ("orchid", 0xda70d6),
    ("tan", 0xd2b48c),
    ("chocolate", 0xd2691e),
    ("skyblue", 0x87ceeb),
    ("steelblue", 0x4682b4),
    ("royalblue", 0x4169e1),
    ("slategray", 0x708090),
    ("lightgray", 0xd3d3d3),
    ("darkgray", 0xa9a9a9),
    ("whitesmoke", 0xf5f5f5),
    ("ivory", 0xfffff0),
    ("mintcream", 0xf5fffa),
    ("seagreen", 0x2e8b57),
    ("forestgreen", 0x228b22),
    ("turquoise", 0x40e0d0),
    ("rebeccapurple", 0x663399),
];

impl serde::Serialize for Color {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for Color {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

impl schemars::JsonSchema for Color {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "Color".into()
    }

    fn json_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "description": "Hex color (#rgb, #rrggbb, #rrggbbaa) or a CSS color name",
            "examples": ["#6366f1", "tomato"]
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_print() {
        assert_eq!("#fff".parse::<Color>().unwrap(), Color::rgb(255, 255, 255));
        assert_eq!("#6366F1".parse::<Color>().unwrap().to_string(), "#6366f1");
        assert_eq!("#00000080".parse::<Color>().unwrap().a, 128);
        assert_eq!("Tomato".parse::<Color>().unwrap(), Color::hex(0xff6347));
        assert!("#12".parse::<Color>().is_err());
        assert!("blurple".parse::<Color>().is_err());
    }

    #[test]
    fn contrast_picks_readable_text() {
        let white = Color::rgb(255, 255, 255);
        let black = Color::rgb(0, 0, 0);
        assert_eq!(Color::hex(0x18181b).readable_on(white, black), white);
        assert_eq!(Color::hex(0xfde68a).readable_on(white, black), black);
    }
}
