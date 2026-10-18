//! Style cascade: theme → layout defaults → branch colors → classes → inline.

use rhizo_core::{Color, Document, Edge, LayoutKind, LineStyle, Node, Shape, Style, Theme};

#[derive(Clone, Debug, PartialEq)]
pub struct NodeLook {
    pub fill: Color,
    pub stroke: Color,
    pub stroke_width: f32,
    pub text: Color,
    pub font_size: f32,
    pub font_weight: u16,
    pub line: LineStyle,
    pub shape: Shape,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EdgeLook {
    pub color: Color,
    pub width: f32,
    pub line: LineStyle,
    pub label_color: Color,
    pub label_size: f32,
}

pub struct NodeCtx {
    pub layout: LayoutKind,
    pub depth: usize,
    pub branch: Option<usize>,
    pub has_children: bool,
}

fn classes<'a>(doc: &'a Document, names: &'a [String]) -> impl Iterator<Item = &'a Style> + 'a {
    names.iter().filter_map(|c| doc.styles.get(c))
}

pub fn node(doc: &Document, theme: &Theme, n: &Node, cx: &NodeCtx) -> NodeLook {
    let mut look = NodeLook {
        fill: theme.node_fill,
        stroke: theme.node_stroke,
        stroke_width: theme.stroke_width,
        text: theme.text,
        font_size: theme.font_size,
        font_weight: theme.font_weight,
        line: LineStyle::Solid,
        shape: Shape::Rounded,
    };
    let tree_like = matches!(cx.layout, LayoutKind::Mindmap | LayoutKind::Tree);
    if tree_like && cx.depth == 0 && cx.has_children {
        look.fill = theme.root_fill;
        look.stroke = theme.root_fill;
        look.text = theme.root_text;
        look.font_size = theme.root_font_size;
        look.font_weight = theme.root_font_weight;
        if cx.layout == LayoutKind::Mindmap {
            look.shape = Shape::Pill;
        }
    } else if cx.layout == LayoutKind::Mindmap {
        if let Some(b) = cx.branch {
            let c = theme.branch(b);
            if cx.depth == 1 {
                look.fill = theme.tint(c, 0.84);
                look.stroke = c;
                look.stroke_width = 1.5;
                look.font_weight = 500;
            } else {
                look.fill = theme.tint(c, 0.94);
                look.stroke = theme.tint(c, 0.55);
            }
        }
    }

    let style = classes(doc, &n.class).fold(Style::default(), |acc, s| acc.overlay(s)).overlay(&n.style);
    let fill_changed = style.fill.is_some();
    apply(&mut look, &style);
    if fill_changed && style.text.is_none() {
        look.text = readable(look.fill, theme);
    }
    look
}

fn apply(look: &mut NodeLook, s: &Style) {
    if let Some(v) = s.fill {
        look.fill = v;
    }
    if let Some(v) = s.stroke {
        look.stroke = v;
    }
    if let Some(v) = s.text {
        look.text = v;
    }
    if let Some(v) = s.stroke_width {
        look.stroke_width = v.max(0.0);
    }
    if let Some(v) = s.font_size {
        look.font_size = v.clamp(6.0, 96.0);
    }
    if let Some(v) = s.font_weight {
        look.font_weight = v;
    }
    if let Some(v) = s.line {
        look.line = v;
    }
    if let Some(v) = s.shape {
        look.shape = v;
    }
}

/// Theme text if it reads well on `fill`, else whichever of the theme's
/// light/dark text colors contrasts more.
fn readable(fill: Color, theme: &Theme) -> Color {
    if fill.contrast(theme.text) >= 4.5 {
        return theme.text;
    }
    let (light, dark) =
        if theme.text.luminance() > 0.5 { (theme.text, theme.background) } else { (theme.background, theme.text) };
    fill.readable_on(light, dark)
}

pub enum EdgeRole {
    /// Parent → child in a mindmap or tree; `depth` is the child's.
    Branch { branch: Option<usize>, depth: usize, mindmap: bool },
    /// Parent → child drawn as an ordinary connection (layered, grid, free).
    Nesting,
    /// An explicit edge.
    Link { secondary: bool },
}

pub fn edge(doc: &Document, theme: &Theme, e: Option<&Edge>, role: EdgeRole) -> EdgeLook {
    let mut look = EdgeLook {
        color: theme.edge,
        width: theme.edge_width,
        line: LineStyle::Solid,
        label_color: theme.muted_text,
        label_size: (theme.font_size - 2.0).max(9.0),
    };
    match role {
        EdgeRole::Branch { branch: Some(b), depth, mindmap: true } => {
            look.color = theme.branch(b);
            look.width = (2.6 - 0.45 * depth.saturating_sub(1) as f32).max(1.25);
        }
        EdgeRole::Branch { .. } | EdgeRole::Nesting => {}
        EdgeRole::Link { secondary } => {
            if secondary {
                look.line = LineStyle::Dashed;
            }
        }
    }
    if let Some(e) = e {
        let style = classes(doc, &e.class).fold(Style::default(), |acc, s| acc.overlay(s)).overlay(&e.style);
        if let Some(c) = style.stroke.or(style.fill) {
            look.color = c;
        }
        if let Some(w) = style.stroke_width {
            look.width = w.max(0.0);
        }
        if let Some(l) = style.line {
            look.line = l;
        }
        if let Some(t) = style.text {
            look.label_color = t;
        }
        if let Some(s) = style.font_size {
            look.label_size = s.clamp(6.0, 96.0);
        }
    }
    look
}
