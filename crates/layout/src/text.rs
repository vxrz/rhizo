/// Measures single lines of text. Renderers with real fonts implement this;
/// [`Heuristic`] is a font-free fallback (WASM, tests).
pub trait TextMeasure {
    fn width(&self, text: &str, font_size: f32, font_weight: u16) -> f32;

    /// Baseline-to-baseline distance.
    fn line_height(&self, font_size: f32) -> f32 {
        (font_size * 1.32).round()
    }
}

/// Approximates a neo-grotesque sans (Inter, Helvetica) with per-class widths.
#[derive(Clone, Copy, Debug, Default)]
pub struct Heuristic;

impl TextMeasure for Heuristic {
    fn width(&self, text: &str, font_size: f32, font_weight: u16) -> f32 {
        let em: f32 = text
            .chars()
            .map(|c| match c {
                'i' | 'l' | 'j' | '.' | ',' | '\'' | '|' | '!' | ':' | ';' => 0.26,
                ' ' | 'f' | 't' | 'r' | 'I' | '(' | ')' | '[' | ']' | '-' => 0.33,
                'm' | 'w' | 'M' | 'W' => 0.86,
                'A'..='Z' => 0.66,
                '0'..='9' => 0.56,
                c if c.is_ascii() => 0.54,
                // CJK and most emoji are full width.
                c if (c as u32) >= 0x2E80 => 1.0,
                _ => 0.58,
            })
            .sum();
        let bold = 1.0 + (font_weight.saturating_sub(400) as f32 / 300.0) * 0.07;
        em * font_size * bold
    }
}

/// Greedy word wrap. Hard line breaks in `text` are kept; words longer than
/// `max_width` are broken by character.
pub fn wrap(text: &str, max_width: f32, size: f32, weight: u16, m: &dyn TextMeasure) -> Vec<String> {
    let mut lines = Vec::new();
    for para in text.split('\n') {
        let mut line = String::new();
        for word in para.split(' ').filter(|w| !w.is_empty()) {
            let candidate = if line.is_empty() { word.to_string() } else { format!("{line} {word}") };
            if m.width(&candidate, size, weight) <= max_width {
                line = candidate;
                continue;
            }
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            if m.width(word, size, weight) <= max_width {
                line = word.to_string();
            } else {
                for c in word.chars() {
                    line.push(c);
                    if m.width(&line, size, weight) > max_width && line.chars().count() > 1 {
                        let last = line.pop().unwrap();
                        lines.push(std::mem::take(&mut line));
                        line.push(last);
                    }
                }
            }
        }
        lines.push(line);
    }
    while lines.len() > 1 && lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wraps_and_keeps_breaks() {
        let m = Heuristic;
        let lines = wrap("the quick brown fox jumps over the lazy dog", 100.0, 14.0, 400, &m);
        assert!(lines.len() > 1);
        assert!(lines.iter().all(|l| m.width(l, 14.0, 400) <= 100.0));
        assert_eq!(wrap("a\nb", 100.0, 14.0, 400, &m), vec!["a", "b"]);
        let long = wrap("supercalifragilisticexpialidocious", 60.0, 14.0, 400, &m);
        assert!(long.len() > 2);
        assert_eq!(wrap("", 100.0, 14.0, 400, &m), vec![""]);
    }
}
