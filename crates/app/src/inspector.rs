//! The inspector, right of the canvas, for what is selected: a node (its
//! label and type, its look, its props as a form made from the type's JSON
//! Schema, its acceptance criteria, where it is defined or used), a frame
//! (its type, layout, direction, look and props), an edge (its kind, line,
//! arrowheads and the props its kits give it) or, with nothing selected,
//! the document (title, theme, kits, the loose nodes' layout). A choice
//! opens a list, a toggle switches, a text field types. Drawn like the
//! sidebar, from the theme background; it scrolls when it runs long.

use rhizo_core::{Color, Props};
use rhizo_kit::TypeDef;
use rhizo_render::Fonts;
use serde_json::Value;
use vello::Scene;
use vello::kurbo::{Affine, Circle, Line, Point, Rect, RoundedRect, Stroke};
use vello::peniko::Fill;

use crate::chrome::{self, ACCENT, ERROR, Text, ink, with_alpha};
use crate::draw::FontCache;
use crate::field::{self, Look};
use crate::peniko_color;
use crate::sidebar::{self, Glyph};

/// Width in logical pixels.
pub const WIDTH: f64 = 300.0;

/// How a prop is edited.
#[derive(Clone, Debug, PartialEq)]
pub enum Input {
    /// One of these values; the palette lists them.
    Choice(Vec<Value>),
    Toggle,
    Number {
        integer: bool,
    },
    Text,
    /// A list or an object: shown here, edited in the text.
    Other,
}

impl Input {
    /// From a prop's schema, else from the value it has.
    fn of(schema: Option<&Value>, value: Option<&Value>) -> Input {
        if let Some(values) = schema.and_then(|s| s.get("enum")).and_then(Value::as_array) {
            return Input::Choice(values.clone());
        }
        match schema.and_then(|s| s.get("type")).and_then(Value::as_str) {
            Some("boolean") => Input::Toggle,
            Some("integer") => Input::Number { integer: true },
            Some("number") => Input::Number { integer: false },
            Some("string") => Input::Text,
            Some(_) => Input::Other,
            None => match value {
                Some(Value::Bool(_)) => Input::Toggle,
                Some(Value::Number(n)) => Input::Number { integer: !n.is_f64() },
                Some(Value::Array(_) | Value::Object(_)) => Input::Other,
                _ if schema.and_then(|s| s.get("type")).is_some() => Input::Other,
                _ => Input::Text,
            },
        }
    }

    /// What typing `text` into the field means: a value, or `None` to remove
    /// the prop (empty text). A number field refuses what is not one.
    pub fn parse(&self, text: &str) -> Result<Option<Value>, String> {
        let t = text.trim();
        if t.is_empty() {
            return Ok(None);
        }
        match self {
            Input::Number { integer: true } => {
                t.parse::<i64>().map(|n| Some(Value::from(n))).map_err(|_| "a whole number".to_string())
            }
            Input::Number { integer: false } => match t.parse::<f64>().ok().and_then(serde_json::Number::from_f64) {
                Some(n) => Ok(Some(t.parse::<i64>().map_or(Value::Number(n), Value::from))),
                None => Err("a number".to_string()),
            },
            _ => Ok(Some(Value::String(text.to_string()))),
        }
    }
}

/// One prop in the form.
#[derive(Clone, Debug, PartialEq)]
pub struct Field {
    /// The prop's key; `azure.resource-group` for one inside an object prop.
    pub key: String,
    /// For a prop inside an object prop: that prop and its own key.
    pub within: Option<(String, String)>,
    pub input: Input,
    /// As written in the file, if set.
    pub value: Option<Value>,
    /// The schema's default, shown faintly while unset.
    pub default: Option<Value>,
    pub required: bool,
    /// Not in the type's schema (an untyped node's props, imported attributes).
    pub extra: bool,
    /// What the schema finds wrong with the value, or the typed text.
    pub problem: Option<String>,
    /// Several nodes are shown and their values differ.
    pub mixed: bool,
}

/// The props after setting (`Some`) or removing (`None`) prop `key`, or
/// the prop inside an object prop that `within` names.
pub fn with_value(props: &Props, within: Option<&(String, String)>, key: &str, v: Option<Value>) -> Props {
    let mut out = props.clone();
    match within {
        None => match v {
            Some(v) => {
                out.insert(key.to_string(), v);
            }
            None => {
                out.shift_remove(key);
            }
        },
        Some((parent, child)) => {
            let mut obj = props.get(parent).and_then(Value::as_object).cloned().unwrap_or_default();
            match v {
                Some(v) => {
                    obj.insert(child.clone(), v);
                }
                None => {
                    obj.remove(child);
                }
            }
            if obj.is_empty() {
                out.shift_remove(parent);
            } else {
                out.insert(parent.clone(), Value::Object(obj));
            }
        }
    }
    out
}

/// An object prop whose keys can each be a field: no key is required (so
/// it can be filled one at a time) and every value is a scalar.
fn flat_object(schema: &Value) -> Option<&serde_json::Map<String, Value>> {
    let props = schema.get("properties")?.as_object()?;
    let scalar = |s: &Value| {
        s.get("enum").is_some()
            || matches!(s.get("type").and_then(Value::as_str), Some("string" | "number" | "integer" | "boolean"))
    };
    (schema.get("type").and_then(Value::as_str) == Some("object")
        && schema.get("required").is_none()
        && props.values().all(scalar))
    .then_some(props)
}

/// The form for `props` of a node or frame of type `def` (`None`: untyped,
/// or a type rhizo does not know): the schema's props in its order, then
/// the others. `typing` is a field being typed in, with what it would become.
pub fn fields(
    def: Option<&TypeDef>,
    props: &Props,
    typing: Option<(&str, Result<Option<Value>, String>)>,
) -> Vec<Field> {
    let problem = |p: &Props, path: &[&str]| def.and_then(|d| d.path_problem(p, path));
    fields_from_schema(def.map(|d| &d.schema), &problem, props, typing)
}

/// A field before its value and problems: key, the object prop it sits in, its schema.
type Slot<'a> = (String, Option<(String, String)>, Option<&'a Value>);

/// [`fields`] from a schema (a type's, or the props an edge takes), with
/// `problem` saying what is wrong at a path of the props.
pub fn fields_from_schema(
    schema: Option<&Value>,
    problem: &dyn Fn(&Props, &[&str]) -> Option<String>,
    props: &Props,
    typing: Option<(&str, Result<Option<Value>, String>)>,
) -> Vec<Field> {
    let declared: Vec<&str> = schema
        .and_then(|s| s.get("properties"))
        .and_then(Value::as_object)
        .map(|o| o.keys().map(String::as_str).collect())
        .unwrap_or_default();
    let required: Vec<&str> = schema
        .and_then(|s| s.get("required"))
        .and_then(Value::as_array)
        .map(|r| r.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let prop_schema = |key: &str| schema.and_then(|s| s.get("properties")).and_then(|p| p.get(key));
    // The fields, before problems: (key, within, schema).
    let mut slots: Vec<Slot> = Vec::new();
    for key in
        declared.iter().map(|k| k.to_string()).chain(props.keys().filter(|k| !declared.contains(&k.as_str())).cloned())
    {
        let ps = prop_schema(&key);
        match ps.and_then(flat_object) {
            Some(inner) => {
                for (child, cs) in inner {
                    slots.push((format!("{key}.{child}"), Some((key.clone(), child.clone())), Some(cs)));
                }
            }
            None => slots.push((key, None, ps)),
        }
    }
    // Problems are found on the props as they would be with the typing in.
    let mut trial = props.clone();
    let mut typing_problem = None;
    if let Some((key, parsed)) = &typing {
        let within = slots.iter().find(|s| s.0 == *key).and_then(|s| s.1.clone());
        match parsed {
            Ok(v) => trial = with_value(props, within.as_ref(), key, v.clone()),
            Err(e) => typing_problem = Some((key.to_string(), format!("needs {e}"))),
        }
    }
    slots
        .into_iter()
        .filter_map(|(key, within, ps)| {
            let value = match &within {
                Some((p, c)) => props.get(p).and_then(|o| o.get(c)).cloned(),
                None => props.get(&key).cloned(),
            };
            let input = Input::of(ps, value.as_ref());
            // A list or object nothing has set yet: nothing to show, and it is written in the text.
            if input == Input::Other && value.is_none() {
                return None;
            }
            let problem = match &typing_problem {
                Some((k, e)) if *k == key => Some(e.clone()),
                _ => match &within {
                    Some((p, c)) => problem(&trial, &[p, c]),
                    None => problem(&trial, &[&key]),
                },
            };
            Some(Field {
                input,
                default: ps.and_then(|s| s.get("default")).cloned(),
                required: within.is_none() && required.contains(&key.as_str()),
                extra: schema.is_some() && ps.is_none(),
                problem,
                mixed: false,
                value,
                within,
                key,
            })
        })
        .collect()
}

/// Marks the fields whose value differs in any of `others` (the other
/// selected nodes' props) as mixed; their problems are not shown then.
pub fn mark_mixed<'a>(fields: &mut [Field], others: impl Iterator<Item = &'a Props> + Clone) {
    for f in fields {
        let of = |p: &'a Props| match &f.within {
            Some((parent, child)) => p.get(parent).and_then(|o| o.get(child)),
            None => p.get(&f.key),
        };
        f.mixed = others.clone().any(|p| of(p) != f.value.as_ref());
        if f.mixed {
            f.problem = None;
        }
    }
}

/// A refusal as a sentence: "cannot adopt `x` yet: …" → "Cannot adopt `x` yet: ….".
fn sentence(why: &str) -> String {
    let mut c = why.chars();
    let first = c.next().map(|f| f.to_uppercase().collect::<String>()).unwrap_or_default();
    let s = first + c.as_str();
    if s.ends_with('.') { s } else { s + "." }
}

/// A value as the form shows it: strings without quotes.
pub fn show(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Where a node is defined, or used, as the panel lists it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum Place {
    /// A reference: the file its node is defined in.
    DefinedIn(String),
    /// Defined here: the other files that use it.
    UsedIn(Vec<String>),
    /// Not part of a project.
    #[default]
    Alone,
    /// Several nodes are shown, this many.
    Selection(usize),
    /// Frames, edges and the document: no references section.
    Hidden,
}

/// Fill colours the look offers: none of its own, then light tints that
/// keep dark text readable (text is chosen for contrast, so they suit the
/// dark theme too).
pub const FILLS: [Option<Color>; 9] = [
    None,
    Some(Color::hex(0xfee2e2)),
    Some(Color::hex(0xfef3c7)),
    Some(Color::hex(0xdcfce7)),
    Some(Color::hex(0xe0f2fe)),
    Some(Color::hex(0xe0e7ff)),
    Some(Color::hex(0xede9fe)),
    Some(Color::hex(0xfce7f3)),
    Some(Color::hex(0xf4f4f5)),
];

/// Stroke colours: none of its own, then strong tones that read on light and dark.
pub const STROKES: [Option<Color>; 9] = [
    None,
    Some(Color::hex(0xef4444)),
    Some(Color::hex(0xf59e0b)),
    Some(Color::hex(0x22c55e)),
    Some(Color::hex(0x0ea5e9)),
    Some(Color::hex(0x6366f1)),
    Some(Color::hex(0x8b5cf6)),
    Some(Color::hex(0xec4899)),
    Some(Color::hex(0x71717a)),
];

/// What the swatches of [`FILLS`] and [`STROKES`] are called in lists.
pub const COLOUR_NAMES: [&str; 9] =
    ["none of its own", "red", "amber", "green", "sky", "indigo", "violet", "pink", "grey"];

/// Swatches of `colors` with `current` lit, if it is one of them.
pub fn swatches(colors: &[Option<Color>], current: Option<Color>) -> SettingKind {
    let chosen = colors.iter().position(|c| *c == current);
    SettingKind::Swatches { colors: colors.to_vec(), chosen }
}

/// What the panel shows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Subject {
    #[default]
    Node,
    Frame,
    Edge,
    /// Nothing selected: the document itself.
    Document,
}

/// A row of the panel's settings: the look, a frame's layout, an edge's
/// kind, the document's title…
#[derive(Clone, Debug, PartialEq)]
pub struct Setting {
    pub name: String,
    /// As shown; empty when unset (then `placeholder` shows, faintly).
    pub value: String,
    pub placeholder: String,
    pub kind: SettingKind,
}

#[derive(Clone, Debug, PartialEq)]
pub enum SettingKind {
    /// A click lists the values.
    Choice,
    /// Typed in.
    Text,
    /// Colour swatches (`None`: no colour of its own); `chosen` is lit.
    Swatches { colors: Vec<Option<Color>>, chosen: Option<usize> },
    /// Shown, not edited here.
    Info,
}

/// What the panel shows for what is selected.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Model {
    pub subject: Subject,
    pub label: String,
    pub id: String,
    /// The type: its name, its label and its kit (`None` for a type no kit defines).
    pub ty: Option<(String, Option<(String, String)>)>,
    pub fields: Vec<Field>,
    pub place: Place,
    /// False for references and non-KDL files: shown, not edited.
    pub editable: bool,
    /// For a node of an imported environment: whether it can switch to
    /// rhizo's code (`None` elsewhere).
    pub adopt: Option<Adoption>,
    /// Look, layout, kind, title…: shown above the props.
    pub settings: Vec<Setting>,
    /// A node's acceptance criteria (`None`: not a node, or several).
    pub accept: Option<Vec<String>>,
    /// A typed frame: what to add in it ("Add typed node…").
    pub add_typed: bool,
}

/// Where a node of imported code stands with adopting.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Adoption {
    /// It can be adopted: the panel offers "Adopt…".
    Ready,
    /// It cannot, and why.
    Refused(String),
    /// An adopt job is running.
    Running,
    /// It is rhizo's code now: its props are edited like any other.
    Adopted,
}

/// What a click on the panel does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hit {
    /// The type row: set or change the type.
    Type,
    /// A field, by its index.
    Field(usize),
    /// A setting, by its index.
    Setting(usize),
    /// Swatch `.1` of setting `.0`.
    Swatch(usize, usize),
    /// An acceptance criterion, to type in.
    Accept(usize),
    AcceptRemove(usize),
    /// "Add criterion".
    AcceptAdd,
    /// "Add typed node…" in a typed frame.
    AddTyped,
    /// Opens the file a reference points into.
    Definition,
    /// A file using the node, by its index.
    Usage(usize),
    InsertReference,
    /// "Adopt…": switch an imported node to rhizo's code.
    Adopt,
}

/// A text being typed in the panel (a field, a setting, a criterion, or
/// `AcceptAdd` for a new one), with its caret and selection.
pub struct Typing<'a> {
    pub at: Hit,
    pub field_text: &'a field::Field,
}

const ROW: f64 = 32.0;

/// Where a field's text starts in a panel at `rect`, and how wide it may run.
pub fn text_span(rect: Rect, dpr: f64) -> (f64, f64) {
    let pad = 16.0 * dpr;
    let key_w = (rect.width() - 2.0 * pad) * 0.4;
    let (x0, x1) = (rect.x0 + pad + key_w, rect.x1 - pad);
    (x0 + 10.0 * dpr, x1 - x0 - 20.0 * dpr)
}

/// What hovering a part of the panel says: the full text of what is cut
/// short, or what a click does and its keys.
pub fn tip(m: &Model, hit: Hit) -> String {
    use crate::palette::{self, Action};
    match hit {
        Hit::Type if m.subject == Subject::Frame => palette::tip(Action::FrameTypeMenu),
        Hit::Type => palette::tip(Action::ChangeTypeMenu),
        Hit::Field(i) => m.fields.get(i).map_or_else(String::new, |f| match (&f.value, &f.input) {
            _ if f.mixed => format!("{}: the selected nodes differ; typing sets it on all of them", f.key),
            (_, Input::Choice(_)) => format!("{}: choose a value", f.key),
            (_, Input::Toggle) => format!("{}: click to switch", f.key),
            (Some(v), _) => format!("{} = {}", f.key, show(v)),
            (None, _) => format!("{}: click to type a value", f.key),
        }),
        Hit::Setting(i) => m.settings.get(i).map_or_else(String::new, |s| match s.kind {
            SettingKind::Choice => format!("{}: choose", s.name),
            SettingKind::Text => format!("{}: click to type", s.name),
            _ => s.name.clone(),
        }),
        Hit::Swatch(i, _) => m.settings.get(i).map_or_else(String::new, |s| s.name.clone()),
        Hit::Accept(i) => m.accept.as_ref().and_then(|a| a.get(i)).cloned().unwrap_or_default(),
        Hit::AcceptRemove(_) => "Remove this criterion".into(),
        Hit::AcceptAdd => palette::tip(Action::AddAccept),
        Hit::AddTyped => palette::tip(Action::AddTypedMenu),
        Hit::Definition => palette::tip(Action::GoToDefinition),
        Hit::Usage(i) => match &m.place {
            Place::UsedIn(files) => files.get(i).map_or_else(String::new, |f| format!("Open {f}")),
            _ => String::new(),
        },
        Hit::InsertReference => palette::tip(Action::InsertReferenceMenu),
        Hit::Adopt => palette::tip(Action::AdoptNode),
    }
}

/// Draws the panel at `rect` for `model`, its content moved up by
/// `scroll` pixels. Returns where each clickable part is, and how tall the
/// content is (to bound scrolling).
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    model: Option<&Model>,
    typing: Option<&Typing>,
    hover: Option<Hit>,
    rect: Rect,
    dpr: f64,
    theme_bg: Color,
    scroll: f64,
) -> (Vec<(Rect, Hit)>, f64) {
    let ink = ink(theme_bg);
    let dark = theme_bg.luminance() < 0.4;
    let bg = theme_bg.mix(ink, if dark { 0.06 } else { 0.04 });
    let px = |v: f64| v * dpr;
    let faint = ink.mix(bg, 0.5);
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg), None, &rect);
    let edge = Line::new((rect.x0 + 0.5 * dpr, rect.y0), (rect.x0 + 0.5 * dpr, rect.y1));
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink.mix(theme_bg, 0.86)), None, &edge);
    s.push_clip_layer(Fill::NonZero, Affine::IDENTITY, &rect);
    let mut hits = Vec::new();
    let pad = px(16.0);
    let (x0, x1) = (rect.x0 + pad, rect.x1 - pad);
    let mut t = Text { s, fonts, cache, size: px(13.0) as f32, baseline: 0.0 };
    let top = rect.y0 + px(14.0) - scroll;
    let mut y = top;

    let Some(m) = model else {
        t.size = px(13.0) as f32;
        t.baseline = (y + px(20.0)) as f32;
        t.draw("Nothing selected", x0, 600, ink);
        t.size = px(12.0) as f32;
        t.baseline = (y + px(42.0)) as f32;
        let hint = t.clip("Click a node to see its type and props", 400, x1 - x0);
        t.draw(&hint, x0, 400, faint);
        t.s.pop_layer();
        return (hits, px(60.0));
    };

    // The label, and the id (or what it is) under it.
    t.size = px(16.0) as f32;
    t.baseline = (y + px(18.0)) as f32;
    let label = t.clip(if m.label.is_empty() { &m.id } else { &m.label }, 600, x1 - x0);
    t.draw(&label, x0, 600, ink);
    t.size = px(11.5) as f32;
    t.baseline = (y + px(36.0)) as f32;
    let id = t.clip(&m.id, 400, x1 - x0);
    t.draw(&id, x0, 400, faint);
    y += px(52.0);

    let key_w = (rect.width() - 2.0 * pad) * 0.4;
    let pill = |s: &mut Scene, r: Rect, c: Color| {
        s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(c), None, &RoundedRect::from_rect(r, px(6.0)))
    };
    let mid_of = |y: f64| y + px(ROW / 2.0);
    let row_rect = |y: f64| Rect::new(rect.x0 + px(8.0), y, rect.x1 - px(8.0), y + px(ROW));
    let editing = |at: Hit| typing.filter(|e| e.at == at);

    // The type: a row that changes it, or a button that sets one.
    if matches!(m.subject, Subject::Node | Subject::Frame) {
        let row = row_rect(y);
        let mid = mid_of(y);
        t.size = px(13.0) as f32;
        t.baseline = (mid + px(13.0) * 0.35) as f32;
        if m.editable && hover == Some(Hit::Type) {
            pill(t.s, row, with_alpha(ink, 0.05));
        }
        t.draw("Type", x0, 400, faint);
        match &m.ty {
            Some((name, known)) => {
                let (label, kit) = known.clone().unwrap_or_else(|| (name.clone(), "unknown kit".into()));
                let label = t.clip(&label, 600, x1 - x0 - key_w);
                t.draw(&label, x0 + key_w, 600, ink);
                t.size = px(11.5) as f32;
                let rest = t.clip(&format!("{name} · {kit}"), 400, x1 - x0 - key_w);
                t.baseline = (mid + px(17.0)) as f32;
                t.draw(&rest, x0 + key_w, 400, faint);
                y += px(10.0);
            }
            None if m.editable => {
                let b = Rect::new(x0 + key_w, mid - px(12.0), x1, mid + px(12.0));
                pill(t.s, b, with_alpha(ACCENT, if hover == Some(Hit::Type) { 0.26 } else { 0.16 }));
                let text = if m.subject == Subject::Frame { "Set frame type…" } else { "Set type…" };
                t.draw(text, b.x0 + px(10.0), 600, ACCENT.mix(ink, 0.25));
            }
            None => {
                t.draw("none", x0 + key_w, 400, faint);
            }
        }
        if m.editable {
            hits.push((Rect::new(row.x0, row.y0, row.x1, y + px(ROW)), Hit::Type));
        }
        y += px(ROW + 8.0);
    }

    let section = |t: &mut Text, y: &mut f64, title: &str| {
        let line = Line::new((x0, *y + 0.5 * dpr), (x1, *y + 0.5 * dpr));
        t.s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(ink.mix(theme_bg, 0.86)), None, &line);
        t.size = px(10.5) as f32;
        t.baseline = (*y + px(22.0)) as f32;
        t.draw(&title.to_uppercase(), x0, 600, faint);
        *y += px(30.0);
    };
    // A box holding a value, as fields and settings draw it.
    let value_box = |t: &mut Text,
                     b: Rect,
                     text: &str,
                     color: Color,
                     edited: Option<&Typing>,
                     placeholder: &str,
                     chevron: bool,
                     border: Color| {
        let fill = if dark { theme_bg.mix(ink, 0.03) } else { Color::hex(0xffffff) };
        let rr = RoundedRect::from_rect(b, px(6.0));
        t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(fill), None, &rr);
        let width = if edited.is_some() { 1.5 } else { 1.0 };
        t.s.stroke(&Stroke::new(px(width)), Affine::IDENTITY, peniko_color(border), None, &rr);
        let room = b.width() - px(20.0) - if chevron { px(14.0) } else { 0.0 };
        let mid = b.center().y;
        match edited {
            Some(e) if e.field_text.text().is_empty() && e.field_text.preedit.is_empty() => {
                // The placeholder shows while the field is empty, the caret before it.
                let text = t.clip(placeholder, 400, room);
                t.draw(&text, b.x0 + px(10.0), 400, ink.mix(bg, 0.6));
                let cx = b.x0 + px(10.0);
                let caret = Line::new((cx, mid - px(8.0)), (cx, mid + px(8.0)));
                t.s.stroke(&Stroke::new(px(1.5)), Affine::IDENTITY, peniko_color(ACCENT), None, &caret);
            }
            Some(e) => {
                let look = Look { size: t.size, weight: 400, ink, focused: true, px: dpr };
                field::draw(t.s, t.fonts, t.cache, e.field_text, b.x0 + px(10.0), t.baseline as f64, room, &look);
            }
            None => {
                let text = t.clip(text, 400, room);
                t.draw(&text, b.x0 + px(10.0), 400, color);
            }
        }
        if chevron {
            sidebar::chevron(t.s, Point::new(b.x1 - px(12.0), mid), true, dpr, faint);
        }
    };

    // Imported code: adopting it into rhizo's code.
    if let Some(a) = &m.adopt {
        section(&mut t, &mut y, "Code");
        let lines = |t: &Text, text: &str| -> Vec<String> {
            // Words into lines that fit; at most four, the last cut short.
            let mut out: Vec<String> = Vec::new();
            for w in text.split(' ') {
                match out.last_mut() {
                    Some(l) if t.width(&format!("{l} {w}"), 400) <= x1 - x0 => *l = format!("{l} {w}"),
                    _ => out.push(w.to_string()),
                }
            }
            if out.len() > 4 {
                let rest = out[3..].join(" ");
                out.truncate(3);
                out.push(t.clip(&rest, 400, x1 - x0));
            }
            out
        };
        t.size = px(12.0) as f32;
        let text = match a {
            Adoption::Ready => {
                "Imported code: its props are read-only. Adopting switches it to rhizo's code, so they become editable."
                    .to_string()
            }
            Adoption::Refused(why) => format!("Imported code: its props are read-only. {}", sentence(why)),
            Adoption::Running => "Adopting… the status bar shows how far it is.".to_string(),
            Adoption::Adopted => "rhizo's code (adopted): its props are editable.".to_string(),
        };
        for l in lines(&t, &text) {
            t.baseline = (y + px(14.0)) as f32;
            t.draw(&l, x0, 400, faint);
            y += px(17.0);
        }
        if *a == Adoption::Ready && m.editable {
            y += px(8.0);
            let b = Rect::new(x0, y, x1, y + px(28.0));
            let hovered = hover == Some(Hit::Adopt);
            pill(t.s, b, with_alpha(ACCENT, if hovered { 0.26 } else { 0.16 }));
            t.size = px(13.0) as f32;
            t.baseline = (b.center().y + px(13.0) * 0.35) as f32;
            t.draw("Adopt…", b.x0 + px(10.0), 600, ACCENT.mix(ink, 0.25));
            hits.push((b, Hit::Adopt));
            y += px(28.0);
        }
        y += px(14.0);
    }

    // Settings: look, layout, kind, title…
    if !m.settings.is_empty() {
        let title = match m.subject {
            Subject::Node => "Look",
            Subject::Frame => "Frame",
            Subject::Edge => "Edge",
            Subject::Document => "Document",
        };
        section(&mut t, &mut y, title);
    }
    for (i, set) in m.settings.iter().enumerate() {
        let mid = mid_of(y);
        t.size = px(13.0) as f32;
        t.baseline = (mid + px(13.0) * 0.35) as f32;
        let name = t.clip(&set.name, 400, key_w - px(14.0));
        t.draw(&name, x0, 400, ink.mix(bg, 0.2));
        let b = Rect::new(x0 + key_w, mid - px(12.0), x1, mid + px(12.0));
        let hovered = hover == Some(Hit::Setting(i)) && m.editable;
        let edited = editing(Hit::Setting(i));
        let border = match () {
            _ if edited.is_some() => ACCENT,
            _ if hovered => ink.mix(bg, 0.6),
            _ => ink.mix(bg, 0.82),
        };
        let (text, color) =
            if set.value.is_empty() { (set.placeholder.as_str(), ink.mix(bg, 0.6)) } else { (set.value.as_str(), ink) };
        match &set.kind {
            SettingKind::Choice | SettingKind::Text => {
                let chevron = set.kind == SettingKind::Choice;
                value_box(&mut t, b, text, color, edited, &set.placeholder, chevron, border);
                if m.editable {
                    hits.push((row_rect(y), Hit::Setting(i)));
                }
            }
            SettingKind::Info => {
                let text = t.clip(text, 400, b.width());
                t.draw(&text, b.x0, 400, color);
            }
            SettingKind::Swatches { colors, chosen } => {
                let n = colors.len().max(1) as f64;
                let step = (b.width() / n).min(px(22.0));
                let r = (step / 2.0 - px(2.0)).min(px(8.0));
                for (j, c) in colors.iter().enumerate() {
                    let center = Point::new(b.x0 + step * (j as f64 + 0.5), mid);
                    let dot = Circle::new(center, r);
                    match c {
                        Some(c) => t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(*c), None, &dot),
                        None => {
                            // No colour of its own: an empty ring with a slash.
                            let slash = Line::new(
                                (center.x - r * 0.6, center.y + r * 0.6),
                                (center.x + r * 0.6, center.y - r * 0.6),
                            );
                            t.s.stroke(&Stroke::new(px(1.2)), Affine::IDENTITY, peniko_color(faint), None, &slash);
                        }
                    }
                    t.s.stroke(&Stroke::new(px(1.0)), Affine::IDENTITY, peniko_color(ink.mix(bg, 0.65)), None, &dot);
                    let lit = *chosen == Some(j);
                    if lit || (hover == Some(Hit::Swatch(i, j)) && m.editable) {
                        let ring = Circle::new(center, r + px(2.5));
                        let c = if lit { ACCENT } else { ink.mix(bg, 0.5) };
                        t.s.stroke(&Stroke::new(px(1.8)), Affine::IDENTITY, peniko_color(c), None, &ring);
                    }
                    if m.editable {
                        let hit =
                            Rect::new(center.x - step / 2.0, mid - px(12.0), center.x + step / 2.0, mid + px(12.0));
                        hits.push((hit, Hit::Swatch(i, j)));
                    }
                }
            }
        }
        y += px(ROW);
    }
    if !m.settings.is_empty() {
        y += px(10.0);
    }

    // The props.
    let typed = m.ty.as_ref().is_some_and(|(_, k)| k.is_some());
    if !m.fields.is_empty() || typed {
        section(&mut t, &mut y, "Props");
    }
    if m.fields.is_empty() && typed {
        t.size = px(12.0) as f32;
        t.baseline = (y + px(14.0)) as f32;
        t.draw("This type takes no props", x0, 400, faint);
        y += px(26.0);
    }
    let mut extra_header = false;
    for (i, f) in m.fields.iter().enumerate() {
        if f.extra && !extra_header {
            extra_header = true;
            y += px(4.0);
            section(&mut t, &mut y, "Not in the schema");
        }
        let mid = mid_of(y);
        t.size = px(13.0) as f32;
        t.baseline = (mid + px(13.0) * 0.35) as f32;
        let key = t.clip(&f.key, 400, key_w - px(14.0));
        let kw = t.draw(&key, x0, 400, ink.mix(bg, 0.2));
        if f.required {
            t.draw("*", x0 + kw + px(2.0), 600, ERROR);
        }
        let b = Rect::new(x0 + key_w, mid - px(12.0), x1, mid + px(12.0));
        let edited = editing(Hit::Field(i));
        let hovered = hover == Some(Hit::Field(i)) && m.editable;
        let border = match () {
            _ if f.problem.is_some() => ERROR,
            _ if edited.is_some() => ACCENT,
            _ if hovered => ink.mix(bg, 0.6),
            _ => ink.mix(bg, 0.82),
        };
        let shown = f.value.as_ref().map(show);
        let unset =
            f.default.as_ref().map(show).unwrap_or_else(|| if f.required { "required".into() } else { String::new() });
        match &f.input {
            Input::Toggle => {
                let on = f.value.as_ref().or(f.default.as_ref()).and_then(Value::as_bool).unwrap_or(false);
                let sw = Rect::new(b.x0, mid - px(9.0), b.x0 + px(32.0), mid + px(9.0));
                let track = if on { ACCENT } else { ink.mix(bg, 0.75) };
                let track = if f.value.is_none() { track.mix(bg, 0.45) } else { track };
                t.s.fill(
                    Fill::NonZero,
                    Affine::IDENTITY,
                    peniko_color(track),
                    None,
                    &RoundedRect::from_rect(sw, px(9.0)),
                );
                let knob = Circle::new((if on { sw.x1 - px(9.0) } else { sw.x0 + px(9.0) }, mid), px(7.0));
                t.s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(Color::hex(0xffffff)), None, &knob);
                let note = if f.value.is_none() {
                    "default"
                } else if on {
                    "yes"
                } else {
                    "no"
                };
                t.size = px(12.0) as f32;
                t.draw(note, sw.x1 + px(8.0), 400, faint);
            }
            input => {
                let (text, color) = match &shown {
                    _ if f.mixed => ("mixed".to_string(), ink.mix(bg, 0.6)),
                    Some(v) if matches!(input, Input::Other) => (v.clone(), faint),
                    Some(v) => (v.clone(), ink),
                    None => (unset.clone(), ink.mix(bg, 0.6)),
                };
                let chevron = matches!(input, Input::Choice(_));
                value_box(&mut t, b, &text, color, edited, &unset, chevron, border);
                // A problem is underlined, wavy like the text pane's.
                if f.problem.is_some() {
                    let room = b.width() - px(20.0);
                    let y = b.y1 - px(4.0);
                    let mut wave = vello::kurbo::BezPath::new();
                    let (mut x, step) = (b.x0 + px(8.0), px(3.0));
                    wave.move_to((x, y));
                    let mut up = true;
                    while x < b.x0 + px(8.0) + room.min(px(120.0)) {
                        x += step;
                        wave.line_to((x, if up { y - px(2.0) } else { y }));
                        up = !up;
                    }
                    t.s.stroke(&Stroke::new(px(1.0)), Affine::IDENTITY, peniko_color(ERROR), None, &wave);
                }
            }
        }
        if m.editable {
            hits.push((row_rect(y), Hit::Field(i)));
        }
        y += px(ROW);
        if let Some(p) = &f.problem {
            t.size = px(11.5) as f32;
            t.baseline = (y + px(6.0)) as f32;
            let p = t.clip(p, 400, x1 - x0 - key_w);
            t.draw(&p, x0 + key_w, 400, ERROR.mix(ink, 0.15));
            y += px(14.0);
        }
    }
    y += px(10.0);

    // A button: "Add typed node…", "Add criterion", "Insert reference…".
    let button = |t: &mut Text, hits: &mut Vec<(Rect, Hit)>, y: &mut f64, text: &str, keys: &str, hit: Hit| {
        let b = Rect::new(x0, *y, x1, *y + px(28.0));
        let hovered = hover == Some(hit);
        pill(t.s, b, with_alpha(ink, if hovered { 0.1 } else { 0.06 }));
        t.size = px(13.0) as f32;
        t.baseline = (b.center().y + px(13.0) * 0.35) as f32;
        t.draw(text, b.x0 + px(10.0), 500, ink);
        if !keys.is_empty() {
            t.size = px(11.5) as f32;
            let keys = chrome::keys(t.fonts, keys);
            let kw = t.width(&keys, 400);
            t.draw(&keys, b.x1 - px(10.0) - kw, 400, faint);
        }
        hits.push((b, hit));
        *y += px(28.0);
    };
    if m.add_typed && m.editable {
        button(&mut t, &mut hits, &mut y, "Add typed node…", "", Hit::AddTyped);
        y += px(14.0);
    }

    // Acceptance criteria: a list to edit.
    if let Some(accept) = &m.accept {
        section(&mut t, &mut y, "Acceptance criteria");
        for (i, a) in accept.iter().enumerate() {
            let mid = mid_of(y);
            t.size = px(13.0) as f32;
            t.baseline = (mid + px(13.0) * 0.35) as f32;
            let x_w = px(24.0);
            let b = Rect::new(x0, mid - px(12.0), x1 - x_w - px(4.0), mid + px(12.0));
            let edited = editing(Hit::Accept(i));
            let border = match () {
                _ if edited.is_some() => ACCENT,
                _ if hover == Some(Hit::Accept(i)) && m.editable => ink.mix(bg, 0.6),
                _ => ink.mix(bg, 0.82),
            };
            value_box(&mut t, b, a, ink, edited, "", false, border);
            if m.editable {
                hits.push((b, Hit::Accept(i)));
                let x = Rect::new(x1 - x_w, mid - px(12.0), x1, mid + px(12.0));
                if hover == Some(Hit::AcceptRemove(i)) {
                    pill(t.s, x, with_alpha(ink, 0.08));
                }
                let c = x.center();
                let d = px(4.5);
                for l in [
                    Line::new((c.x - d, c.y - d), (c.x + d, c.y + d)),
                    Line::new((c.x - d, c.y + d), (c.x + d, c.y - d)),
                ] {
                    t.s.stroke(&Stroke::new(px(1.4)), Affine::IDENTITY, peniko_color(faint), None, &l);
                }
                hits.push((x, Hit::AcceptRemove(i)));
            }
            y += px(ROW);
        }
        if let Some(e) = editing(Hit::AcceptAdd) {
            let mid = mid_of(y);
            t.size = px(13.0) as f32;
            t.baseline = (mid + px(13.0) * 0.35) as f32;
            let b = Rect::new(x0, mid - px(12.0), x1, mid + px(12.0));
            value_box(&mut t, b, "", ink, Some(e), "What must hold…", false, ACCENT);
            y += px(ROW);
        } else if accept.is_empty() {
            t.size = px(12.0) as f32;
            t.baseline = (y + px(14.0)) as f32;
            t.draw("None yet", x0, 400, faint);
            y += px(24.0);
        }
        if m.editable {
            y += px(4.0);
            button(&mut t, &mut hits, &mut y, "Add criterion", "", Hit::AcceptAdd);
        }
        y += px(14.0);
    }

    // Where it is defined, or used.
    if m.place != Place::Hidden {
        section(&mut t, &mut y, "References");
        let link =
            |t: &mut Text, hits: &mut Vec<(Rect, Hit)>, y: &mut f64, g: Glyph, text: &str, action: &str, hit: Hit| {
                let r = Rect::new(rect.x0 + px(8.0), *y, rect.x1 - px(8.0), *y + px(28.0));
                if hover == Some(hit) {
                    pill(t.s, r, with_alpha(ink, 0.05));
                }
                let mid = r.center().y;
                sidebar::glyph(t.s, g, Point::new(x0 + px(7.0), mid), dpr, theme_bg);
                t.size = px(13.0) as f32;
                t.baseline = (mid + px(13.0) * 0.35) as f32;
                let aw = t.width(action, 600);
                let text = t.clip(text, 400, x1 - x0 - px(22.0) - aw - px(8.0));
                t.draw(&text, x0 + px(22.0), 400, ink);
                t.draw(action, x1 - aw, 600, ACCENT.mix(ink, 0.2));
                hits.push((r, hit));
                *y += px(28.0);
            };
        let note = |t: &mut Text, y: &mut f64, text: &str| {
            t.size = px(12.0) as f32;
            t.baseline = (*y + px(14.0)) as f32;
            let text = t.clip(text, 400, x1 - x0);
            t.draw(&text, x0, 400, faint);
            *y += px(24.0);
        };
        match &m.place {
            Place::DefinedIn(file) => link(
                &mut t,
                &mut hits,
                &mut y,
                Glyph::Reference,
                &format!("Defined in {file}"),
                "Open",
                Hit::Definition,
            ),
            Place::UsedIn(files) if files.is_empty() => note(&mut t, &mut y, "Not used in other files"),
            Place::UsedIn(files) => {
                note(&mut t, &mut y, &format!("Used in {}", chrome::plural(files.len(), "file")));
                for (i, f) in files.iter().enumerate() {
                    link(&mut t, &mut hits, &mut y, Glyph::File, f, "Open", Hit::Usage(i));
                }
            }
            Place::Alone => note(&mut t, &mut y, "Not part of a project"),
            Place::Selection(n) => note(&mut t, &mut y, &format!("{n} nodes selected: a change applies to all")),
            Place::Hidden => {}
        }
        if m.editable {
            y += px(6.0);
            button(&mut t, &mut hits, &mut y, "Insert reference…", "⌘R", Hit::InsertReference);
        }
    }
    t.s.pop_layer();
    // Parts scrolled out of the panel cannot be clicked.
    hits.retain(|(r, _)| r.y1 > rect.y0 && r.y0 < rect.y1);
    (hits, y - top + px(24.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_form_follows_the_schema() {
        let kits = rhizo_kit::Kits::builtin();
        let op = kits.find_type("data.operation").unwrap();
        let mut props = Props::new();
        props.insert("path".into(), json!("orders"));
        props.insert("x-owner".into(), json!("shop team"));
        let f = fields(Some(op), &props, None);
        let keys: Vec<&str> = f.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(keys, ["method", "path", "summary", "description", "deprecated", "x-owner"]);
        assert!(matches!(&f[0].input, Input::Choice(v) if v.contains(&json!("post"))));
        assert!(f[0].required && f[1].required && !f[2].required);
        assert_eq!(f[1].input, Input::Text);
        assert!(f[1].problem.as_deref().is_some_and(|p| p.contains("^/")), "path must start with /");
        assert_eq!((f[4].input.clone(), f[4].default.clone()), (Input::Toggle, Some(json!(false))));
        assert!(f[5].extra && f[5].input == Input::Text);
        // Typing a fix clears the problem before it is written.
        let f = fields(Some(op), &props, Some(("path", Input::Text.parse("/orders"))));
        assert_eq!(f[1].problem, None);
        // Untyped: every prop by its value, none extra.
        let f = fields(None, &props, None);
        assert!(f.iter().all(|f| !f.extra && f.input == Input::Text));
    }

    #[test]
    fn several_nodes_show_mixed_where_they_differ() {
        let kits = rhizo_kit::Kits::builtin();
        let op = kits.find_type("data.operation").unwrap();
        let props = |method: &str, path: &str| {
            let mut p = Props::new();
            p.insert("method".into(), json!(method));
            p.insert("path".into(), json!(path));
            p
        };
        let (a, b, c) = (props("post", "/orders"), props("post", "/carts"), props("post", "/orders"));
        let mut f = fields(Some(op), &a, None);
        mark_mixed(&mut f, [&b, &c].into_iter());
        let mixed: Vec<&str> = f.iter().filter(|f| f.mixed).map(|f| f.key.as_str()).collect();
        assert_eq!(mixed, ["path"], "the method is the same, the paths differ");
        let mut f = fields(Some(op), &a, None);
        mark_mixed(&mut f, [&c].into_iter());
        assert!(f.iter().all(|f| !f.mixed), "the same props: nothing mixed");
        let summary = f.iter().find(|f| f.key == "summary").unwrap();
        let mut d = c.clone();
        d.insert("summary".into(), json!("Place it"));
        let mut f = fields(Some(op), &a, None);
        mark_mixed(&mut f, [&d].into_iter());
        assert!(f.iter().find(|f| f.key == "summary").unwrap().mixed, "set on one, unset on the other");
        assert!(!summary.mixed);
    }

    #[test]
    fn typed_text_becomes_a_value() {
        assert_eq!(Input::Text.parse("  "), Ok(None), "empty removes");
        assert_eq!(Input::Text.parse("a b"), Ok(Some(json!("a b"))));
        assert_eq!(Input::Number { integer: true }.parse("30"), Ok(Some(json!(30))));
        assert!(Input::Number { integer: true }.parse("3.5").is_err());
        assert_eq!(Input::Number { integer: false }.parse("3.5"), Ok(Some(json!(3.5))));
        assert_eq!(Input::Number { integer: false }.parse("2"), Ok(Some(json!(2))));
        assert!(Input::Number { integer: false }.parse("lots").is_err());
        let kits = rhizo_kit::Kits::builtin();
        let lambda = kits.find_type("aws.lambda.function").unwrap();
        let f = fields(Some(lambda), &Props::new(), Some(("memory-mb", Err("a whole number".into()))));
        let mem = f.iter().find(|f| f.key == "memory-mb").unwrap();
        assert_eq!(mem.problem.as_deref(), Some("needs a whole number"));
        assert!(f.iter().all(|f| f.key != "environment"), "an unset object without fields is left to the text");
        let mut props = Props::new();
        props.insert("environment".into(), json!({ "A": "1" }));
        let f = fields(Some(lambda), &props, None);
        assert_eq!(f.iter().find(|f| f.key == "environment").unwrap().input, Input::Other);
    }

    #[test]
    fn an_environment_shows_cloud_region_budget_and_the_cloud_blocks() {
        let kits = rhizo_kit::Kits::builtin();
        let env = kits.find_type("infra.environment").unwrap();
        let mut props = Props::new();
        props.insert("cloud".into(), json!("azure"));
        props.insert("azure".into(), json!({ "resource-group": "rg-shop" }));
        let f = fields(Some(env), &props, None);
        let keys: Vec<&str> = f.iter().map(|f| f.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "cloud",
                "region",
                "state-bucket",
                "azure.resource-group",
                "azure.state-storage-account",
                "azure.state-container",
                "azure.state-resource-group",
                "gcp.state-bucket",
                "gcp.state-prefix",
                "budget-monthly",
                "lock-file"
            ],
            "origin and blocks are imports' business: hidden until set"
        );
        assert!(matches!(&f[0].input, Input::Choice(v) if v.len() == 3));
        assert_eq!(f[3].value, Some(json!("rg-shop")));
        assert_eq!(f[3].within, Some(("azure".into(), "resource-group".into())));
        assert_eq!(f[5].default, Some(json!("tfstate")));
        assert_eq!(f[9].input, Input::Number { integer: false });
        // A nested value is checked where it is.
        let typing = Some(("azure.state-storage-account", Input::Text.parse("Not Valid!")));
        let f = fields(Some(env), &props, typing);
        assert!(f[4].problem.is_some(), "{f:#?}");
        // Setting and clearing one inside the block.
        let set = with_value(&props, f[4].within.as_ref(), &f[4].key, Some(json!("shopstate")));
        assert_eq!(set["azure"], json!({ "resource-group": "rg-shop", "state-storage-account": "shopstate" }));
        let cleared = with_value(&props, f[3].within.as_ref(), &f[3].key, None);
        assert!(!cleared.contains_key("azure"), "the last key takes its block along");
    }

    #[test]
    fn edge_props_come_from_the_kits() {
        let schema = rhizo_kit::edge_schema(Some("infra.function"), Some("infra.bucket"));
        let f = fields_from_schema(schema.as_ref(), &|_, _| None, &Props::new(), None);
        assert_eq!(f.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), ["access", "prefix"]);
        assert!(matches!(&f[0].input, Input::Choice(v) if v.contains(&json!("read-write"))));
    }
}
