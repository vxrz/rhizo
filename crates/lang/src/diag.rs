use std::fmt;

use miette::{LabeledSpan, NamedSource, SourceSpan};

use crate::ast::Span;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Warning,
    Error,
}

/// A problem found while reading a document, with an optional position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    pub message: String,
    pub span: Span,
    pub label: Option<String>,
    pub help: Option<String>,
}

impl Diagnostic {
    pub fn error(message: impl Into<String>, span: Span) -> Self {
        Diagnostic { severity: Severity::Error, message: message.into(), span, label: None, help: None }
    }

    pub fn warning(message: impl Into<String>, span: Span) -> Self {
        Diagnostic { severity: Severity::Warning, ..Diagnostic::error(message, span) }
    }

    pub fn with_help(mut self, help: impl Into<String>) -> Self {
        self.help = Some(help.into());
        self
    }

    pub fn with_label(mut self, label: impl Into<String>) -> Self {
        self.label = Some(label.into());
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }

    /// 1-based line and column of the span start in `src`.
    pub fn line_col(&self, src: &str) -> Option<(usize, usize)> {
        let (offset, _) = self.span?;
        let before = src.get(..offset.min(src.len()))?;
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
        Some((line, col))
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Diagnostic {}

impl miette::Diagnostic for Diagnostic {
    fn severity(&self) -> Option<miette::Severity> {
        Some(match self.severity {
            Severity::Error => miette::Severity::Error,
            Severity::Warning => miette::Severity::Warning,
        })
    }

    fn help<'a>(&'a self) -> Option<Box<dyn fmt::Display + 'a>> {
        self.help.as_ref().map(|h| Box::new(h) as Box<dyn fmt::Display>)
    }

    fn labels(&self) -> Option<Box<dyn Iterator<Item = LabeledSpan> + '_>> {
        let (offset, len) = self.span?;
        let span = SourceSpan::new(offset.into(), len);
        Some(Box::new(std::iter::once(LabeledSpan::new_with_span(self.label.clone(), span))))
    }
}

/// All diagnostics for one source file, printable with miette's fancy handler.
#[derive(Debug, thiserror::Error, miette::Diagnostic)]
#[error("{summary}")]
pub struct Report {
    summary: String,
    #[source_code]
    src: NamedSource<String>,
    #[related]
    diagnostics: Vec<Diagnostic>,
}

impl Report {
    pub fn new(name: &str, src: &str, diagnostics: Vec<Diagnostic>) -> Self {
        let errors = diagnostics.iter().filter(|d| d.is_error()).count();
        let warnings = diagnostics.len() - errors;
        let plural = |n: usize, w: &str| if n == 1 { format!("1 {w}") } else { format!("{n} {w}s") };
        let summary = match (errors, warnings) {
            (0, w) => format!("{name}: {}", plural(w, "warning")),
            (e, 0) => format!("{name}: {}", plural(e, "error")),
            (e, w) => format!("{name}: {}, {}", plural(e, "error"), plural(w, "warning")),
        };
        Report { summary, src: NamedSource::new(name, src.to_string()), diagnostics }
    }
}

/// Optimal string alignment distance (Levenshtein plus adjacent swaps),
/// case-insensitive, for "did you mean" hints.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let mut d = vec![vec![0usize; b.len() + 1]; a.len() + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for (j, cell) in d[0].iter_mut().enumerate() {
        *cell = j;
    }
    for i in 1..=a.len() {
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            d[i][j] = (d[i - 1][j] + 1).min(d[i][j - 1] + 1).min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                d[i][j] = d[i][j].min(d[i - 2][j - 2] + 1);
            }
        }
    }
    d[a.len()][b.len()]
}

/// Like [`suggest`], but for names the user invents (node ids and labels):
/// very short names are too easy to confuse, so they never match.
pub(crate) fn suggest_name<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let len = word.chars().count();
    if len <= 3 {
        return None;
    }
    let max = if len <= 6 { 1 } else { 2 };
    let skeleton = |s: &str| s.chars().filter(|c| !c.is_ascii_digit()).collect::<String>();
    let word_skeleton = skeleton(word);
    candidates
        .into_iter()
        .filter(|c| c.chars().count() > 3)
        // `step10` vs `step11` is a numbered series, not a typo.
        .filter(|c| !(c.chars().any(|ch| ch.is_ascii_digit()) && skeleton(c) == word_skeleton))
        .map(|c| (edit_distance(word, c), c))
        .filter(|(d, _)| *d <= max)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}

/// The closest candidate within a small edit distance.
pub fn suggest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let max = (word.chars().count() / 3).max(1);
    candidates
        .into_iter()
        .map(|c| (edit_distance(word, c), c))
        .filter(|(d, _)| *d <= max)
        .min_by_key(|(d, _)| *d)
        .map(|(_, c)| c)
}
