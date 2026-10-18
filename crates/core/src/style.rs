use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::Color;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Shape {
    Rect,
    #[default]
    Rounded,
    Pill,
    Ellipse,
    Circle,
    Diamond,
    Hexagon,
    /// Just the label, no outline.
    Text,
}

impl Shape {
    pub const ALL: [Shape; 8] = [
        Shape::Rect,
        Shape::Rounded,
        Shape::Pill,
        Shape::Ellipse,
        Shape::Circle,
        Shape::Diamond,
        Shape::Hexagon,
        Shape::Text,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Shape::Rect => "rect",
            Shape::Rounded => "rounded",
            Shape::Pill => "pill",
            Shape::Ellipse => "ellipse",
            Shape::Circle => "circle",
            Shape::Diamond => "diamond",
            Shape::Hexagon => "hexagon",
            Shape::Text => "text",
        }
    }

    pub fn parse(s: &str) -> Option<Shape> {
        Some(match s {
            "rect" | "box" | "rectangle" | "square" => Shape::Rect,
            "rounded" | "round" => Shape::Rounded,
            "pill" | "stadium" | "capsule" => Shape::Pill,
            "ellipse" | "oval" => Shape::Ellipse,
            "circle" => Shape::Circle,
            "diamond" | "rhombus" | "decision" => Shape::Diamond,
            "hexagon" => Shape::Hexagon,
            "text" | "none" | "plain" => Shape::Text,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum Arrow {
    None,
    #[default]
    End,
    Start,
    Both,
}

impl Arrow {
    pub fn name(self) -> &'static str {
        match self {
            Arrow::None => "none",
            Arrow::End => "end",
            Arrow::Start => "start",
            Arrow::Both => "both",
        }
    }

    pub fn parse(s: &str) -> Option<Arrow> {
        Some(match s {
            "none" | "false" => Arrow::None,
            "end" | "forward" | "true" => Arrow::End,
            "start" | "back" | "backward" => Arrow::Start,
            "both" => Arrow::Both,
            _ => return None,
        })
    }

    pub fn at_end(self) -> bool {
        matches!(self, Arrow::End | Arrow::Both)
    }

    pub fn at_start(self) -> bool {
        matches!(self, Arrow::Start | Arrow::Both)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LineStyle {
    #[default]
    Solid,
    Dashed,
    Dotted,
}

impl LineStyle {
    pub fn name(self) -> &'static str {
        match self {
            LineStyle::Solid => "solid",
            LineStyle::Dashed => "dashed",
            LineStyle::Dotted => "dotted",
        }
    }

    pub fn parse(s: &str) -> Option<LineStyle> {
        Some(match s {
            "solid" => LineStyle::Solid,
            "dashed" | "dash" => LineStyle::Dashed,
            "dotted" | "dot" => LineStyle::Dotted,
            _ => return None,
        })
    }
}

/// Visual overrides. Every field is optional: unset fields fall through to
/// classes, then to the theme. Used both for named styles (`style "accent"`)
/// and inline on nodes and edges.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case", deny_unknown_fields)]
pub struct Style {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke: Option<Color>,
    /// Text color.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_weight: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<LineStyle>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<Shape>,
}

impl Style {
    pub fn is_empty(&self) -> bool {
        *self == Style::default()
    }

    /// Fields set in `top` win over fields set in `self`.
    pub fn overlay(&self, top: &Style) -> Style {
        Style {
            fill: top.fill.or(self.fill),
            stroke: top.stroke.or(self.stroke),
            text: top.text.or(self.text),
            stroke_width: top.stroke_width.or(self.stroke_width),
            font_size: top.font_size.or(self.font_size),
            font_weight: top.font_weight.or(self.font_weight),
            line: top.line.or(self.line),
            shape: top.shape.or(self.shape),
        }
    }
}
