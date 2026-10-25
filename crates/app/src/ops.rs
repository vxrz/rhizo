//! What the inspector's settings and forms do to the file, as functions of
//! the [`Editor`]: the viewer runs them inside one undoable edit, and tests
//! run them as they are. Each setting has a name (the row's label); that
//! name is how the panel, the commands and the parity table refer to it.

use rhizo_core::{
    Arrow, Color, Direction, Document, EdgeKind, Frame, FrameId, LayoutKind, LineStyle, Node, NodeId, Shape,
};
use rhizo_kit::Kits;
use rhizo_lang::{EditError, Editor};
use serde_json::Value;

use crate::inspector::{self, FILLS, Model, Place, STROKES, Setting, SettingKind, Subject};

/// Label of a node just made, until it is typed over.
pub const NEW_LABEL: &str = "New idea";

/// What a form or a setting writes to.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    Nodes(Vec<NodeId>),
    Frame(FrameId),
    /// An index into the document's edges.
    Edge(usize),
    Document,
}

// Setting names: the rows of the panel.
pub const LABEL: &str = "Label";
pub const SHAPE: &str = "Shape";
pub const FILL: &str = "Fill";
pub const STROKE: &str = "Stroke";
pub const NOTE: &str = "Note";
pub const LINK: &str = "Link";
pub const TAGS: &str = "Tags";
pub const CLASS: &str = "Class";
pub const LAYOUT: &str = "Layout";
pub const DIRECTION: &str = "Direction";
pub const KIND: &str = "Kind";
pub const LINE: &str = "Line";
pub const ARROW: &str = "Arrowheads";
pub const PREDICATE: &str = "Predicate";
pub const TITLE: &str = "Title";
pub const THEME: &str = "Theme";
pub const KITS: &str = "Kits";
pub const IMPORTS: &str = "Imports";

const LAYOUTS: [LayoutKind; 8] = [
    LayoutKind::Auto,
    LayoutKind::Mindmap,
    LayoutKind::Tree,
    LayoutKind::Layered,
    LayoutKind::Grid,
    LayoutKind::Force,
    LayoutKind::Free,
    LayoutKind::Sequence,
];
const DIRECTIONS: [Direction; 4] = [Direction::Right, Direction::Down, Direction::Left, Direction::Up];

fn row(name: &str, value: impl Into<String>, placeholder: &str, kind: SettingKind) -> Setting {
    Setting { name: name.into(), value: value.into(), placeholder: placeholder.into(), kind }
}

/// The look of a node: shape, colours, note, link, tags and classes.
pub fn node_settings(n: &Node) -> Vec<Setting> {
    let s = &n.style;
    vec![
        row(SHAPE, s.shape.map(Shape::name).unwrap_or_default(), "default", SettingKind::Choice),
        row(FILL, "", "", inspector::swatches(&FILLS, s.fill)),
        row(STROKE, "", "", inspector::swatches(&STROKES, s.stroke)),
        row(NOTE, n.note.clone().unwrap_or_default(), "", SettingKind::Text),
        row(LINK, n.link.clone().unwrap_or_default(), "", SettingKind::Text),
        row(TAGS, n.tags.join(" "), "", SettingKind::Text),
        row(CLASS, n.class.join(" "), "", SettingKind::Text),
    ]
}

/// A frame's layout, direction and colours.
pub fn frame_settings(f: &Frame) -> Vec<Setting> {
    let layout = if f.layout == LayoutKind::Auto { String::new() } else { f.layout.name().to_string() };
    vec![
        row(LAYOUT, layout, "auto", SettingKind::Choice),
        row(DIRECTION, f.direction.map(Direction::name).unwrap_or_default(), "default", SettingKind::Choice),
        row(FILL, "", "", inspector::swatches(&FILLS, f.style.fill)),
        row(STROKE, "", "", inspector::swatches(&STROKES, f.style.stroke)),
    ]
}

/// An edge's label, kind, line, arrowheads, colour and predicate.
pub fn edge_settings(e: &rhizo_core::Edge) -> Vec<Setting> {
    let arrow = if e.arrow == Arrow::End { String::new() } else { e.arrow.name().to_string() };
    vec![
        row(LABEL, e.label.clone().unwrap_or_default(), "", SettingKind::Text),
        row(KIND, e.kind.map(EdgeKind::name).unwrap_or_default(), "none", SettingKind::Choice),
        row(LINE, e.style.line.map(LineStyle::name).unwrap_or_default(), "solid", SettingKind::Choice),
        row(ARROW, arrow, "end", SettingKind::Choice),
        row(STROKE, "", "", inspector::swatches(&STROKES, e.style.stroke)),
        row(
            PREDICATE,
            e.ty.as_ref().map(|t| t.to_string()).unwrap_or_default(),
            "e.g. skos:broader",
            SettingKind::Text,
        ),
    ]
}

/// The document: title, theme, kits, imports, and the layout of the nodes
/// outside frames (when there are any).
pub fn document_settings(doc: &Document) -> Vec<Setting> {
    let kits: Vec<String> = doc.kits.iter().map(|k| k.kit.clone()).collect();
    let mut out = vec![
        row(TITLE, doc.title.clone().unwrap_or_default(), "Untitled", SettingKind::Text),
        row(THEME, doc.theme.clone().unwrap_or_default(), "light", SettingKind::Choice),
        row(KITS, kits.join(", "), "none", SettingKind::Choice),
    ];
    if let Some(f) = doc.frames.iter().find(|f| f.implicit) {
        let mut rows = frame_settings(f);
        rows.truncate(2);
        out.extend(rows);
    }
    let imports: Vec<String> = doc.imports.iter().map(|(a, p)| format!("{a} = {p}")).collect();
    if !imports.is_empty() {
        out.push(row(IMPORTS, imports.join(", "), "", SettingKind::Info));
    }
    out
}

/// The values a choice setting lists, as written, with what each reads as;
/// the first is "not set" where the setting can be unset.
pub fn choices(name: &str) -> Vec<(Option<&'static str>, &'static str)> {
    match name {
        SHAPE => {
            std::iter::once((None, "default")).chain(Shape::ALL.iter().map(|s| (Some(s.name()), s.name()))).collect()
        }
        LAYOUT => LAYOUTS.iter().map(|l| ((*l != LayoutKind::Auto).then_some(l.name()), l.name())).collect(),
        DIRECTION => std::iter::once((None, "the layout's own"))
            .chain(DIRECTIONS.iter().map(|d| (Some(d.name()), d.name())))
            .collect(),
        KIND => {
            [(None, "none"), (Some("data"), "data"), (Some("control"), "control"), (Some("dependency"), "dependency")]
                .to_vec()
        }
        LINE => [(None, "solid"), (Some("dashed"), "dashed"), (Some("dotted"), "dotted")].to_vec(),
        ARROW => [(None, "end"), (Some("start"), "start"), (Some("both"), "both"), (Some("none"), "none")].to_vec(),
        THEME => std::iter::once((None, "light (default)"))
            .chain(rhizo_core::Theme::NAMES.iter().filter(|n| **n != "light").map(|n| (Some(*n), *n)))
            .collect(),
        _ => Vec::new(),
    }
}

/// Sets (`Some`) or clears (`None`) setting `name` of `owner`. Colours
/// are `#rrggbb`; tags and classes are separated by spaces.
pub fn apply_setting(e: &mut Editor, owner: &Owner, name: &str, value: Option<&str>) -> Result<(), EditError> {
    let value = value.map(str::trim).filter(|v| !v.is_empty());
    let text = |v: &str| Value::String(v.to_string());
    let words = |v: &str| v.split([' ', ',']).filter(|w| !w.is_empty()).collect::<Vec<_>>().join(" ");
    let bad = || EditError::Invalid(format!("`{name}` is not set here"));
    match owner {
        Owner::Nodes(ids) => {
            let key = match name {
                SHAPE => "shape",
                FILL => "fill",
                STROKE => "stroke",
                NOTE => "note",
                LINK => "link",
                TAGS => "tags",
                CLASS => "class",
                _ => return Err(bad()),
            };
            let value = value.map(|v| if matches!(name, TAGS | CLASS) { words(v) } else { v.to_string() });
            ids.iter().try_for_each(|id| match &value {
                Some(v) => e.set_look(id, key, &text(v)),
                None => e.remove_look(id, key),
            })
        }
        Owner::Frame(f) => match name {
            LAYOUT => e.set_frame_layout(f, value.and_then(LayoutKind::parse).filter(|l| *l != LayoutKind::Auto)),
            DIRECTION => e.set_frame_direction(f, value.and_then(Direction::parse)),
            FILL => e.set_frame_look(f, "fill", value.map(text).as_ref()),
            STROKE => e.set_frame_look(f, "stroke", value.map(text).as_ref()),
            LABEL => e.rename_frame(f, value.unwrap_or_default()),
            _ => Err(bad()),
        },
        Owner::Edge(k) => match name {
            LABEL => e.set_edge_label(*k, value),
            KIND => e.set_edge_attr(*k, "kind", value),
            LINE => e.set_edge_attr(*k, "line", value),
            ARROW => e.set_edge_attr(*k, "arrow", value),
            STROKE => e.set_edge_attr(*k, "stroke", value),
            PREDICATE => e.set_edge_attr(*k, "type", value),
            _ => Err(bad()),
        },
        Owner::Document => match name {
            TITLE => e.set_title(value),
            THEME => e.set_theme(value),
            LAYOUT => e.set_top_layout(value.and_then(LayoutKind::parse).filter(|l| *l != LayoutKind::Auto)),
            DIRECTION => e.set_top_direction(value.and_then(Direction::parse)),
            _ => Err(bad()),
        },
    }
}

/// A colour as the file writes it.
pub fn hex(c: Color) -> String {
    c.to_hex_rgb()
}

/// Sets (or, `None`, removes) prop `key` of `owner`; `within` names the
/// object prop it sits in (`azure` for `azure.resource-group`).
pub fn set_prop(
    e: &mut Editor,
    owner: &Owner,
    within: Option<&(String, String)>,
    key: &str,
    value: Option<Value>,
) -> Result<(), EditError> {
    // The prop actually written: the key, or the object holding it.
    let write = |props: &rhizo_core::Props| -> (String, Option<Value>) {
        match within {
            None => (key.to_string(), value.clone()),
            Some((parent, _)) => {
                let after = inspector::with_value(props, within, key, value.clone());
                (parent.clone(), after.get(parent).cloned())
            }
        }
    };
    match owner {
        Owner::Nodes(ids) => ids.iter().try_for_each(|id| {
            let props = e.document().node(id).map(|n| n.props.clone()).unwrap_or_default();
            match write(&props) {
                (k, Some(v)) => e.set_prop(id, &k, &v),
                (k, None) => e.remove_prop(id, &k),
            }
        }),
        Owner::Frame(f) => {
            let props = e.document().frame(f).map(|f| f.props.clone()).unwrap_or_default();
            match write(&props) {
                (k, Some(v)) => e.set_frame_prop(f, &k, &v),
                (k, None) => e.remove_frame_prop(f, &k),
            }
        }
        Owner::Edge(i) => {
            let props = e.document().edges.get(*i).map(|x| x.props.clone()).unwrap_or_default();
            match write(&props) {
                (k, Some(v)) => e.set_edge_prop(*i, &k, &v),
                (k, None) => e.remove_edge_prop(*i, &k),
            }
        }
        Owner::Document => Err(EditError::Invalid("the document has no props".into())),
    }
}

/// The kit to `use` for type `ty`, with the version to ask for; none for
/// the built-in `rhizo` kit, which every document has.
pub fn kit_use(kits: &Kits, ty: &str) -> Option<(String, String)> {
    let (kit, _) = kits.owner(ty)?;
    (kit.name != rhizo_kit::IMPLICIT)
        .then(|| (kit.name.clone(), format!("{}.{}", kit.version.major, kit.version.minor)))
}

/// Types nodes (or, `None`, untypes them), adding the kit's `use` line.
pub fn set_type(e: &mut Editor, kits: &Kits, ids: &[NodeId], ty: Option<&str>) -> Result<(), EditError> {
    if let Some((k, v)) = ty.and_then(|t| kit_use(kits, t)) {
        e.use_kit(&k, &v)?;
    }
    ids.iter().try_for_each(|id| e.set_type(id, ty))
}

/// Types a frame (an environment, an API, a workflow…), adding the kit's `use` line.
pub fn set_frame_type(e: &mut Editor, kits: &Kits, frame: &FrameId, ty: Option<&str>) -> Result<(), EditError> {
    if let Some((k, v)) = ty.and_then(|t| kit_use(kits, t)) {
        e.use_kit(&k, &v)?;
    }
    e.set_frame_type(frame, ty)
}

/// The frame types the kits define, as (name, label, kit).
pub fn frame_types(kits: &Kits) -> Vec<(&str, &str, &str)> {
    kits.iter()
        .flat_map(|k| {
            k.types.values().filter(|d| d.frame).map(move |d| (d.name.as_str(), d.label.as_str(), k.name.as_str()))
        })
        .collect()
}

/// The type a new node in `frame` sits under, for [`rhizo_kit::TypeDef::fits`].
pub fn container_of_frame<'a>(doc: &'a Document, frame: &FrameId) -> Option<&'a str> {
    doc.frame(frame)?.ty.as_ref().map(|t| t.as_str())
}

/// Where "Add typed node…" puts the node.
pub enum Spot {
    /// Right after this node, like Enter.
    After(NodeId),
    /// Last in this frame.
    In(FrameId),
}

/// A node of type `ty` (with its kit's `use` line), labelled
/// [`NEW_LABEL`] until its label is typed ([`Editor::rename_new`]).
pub fn add_typed(e: &mut Editor, kits: &Kits, at: &Spot, ty: &str) -> Result<NodeId, EditError> {
    let id = match at {
        Spot::After(n) => e.add_after(n, NEW_LABEL)?,
        Spot::In(f) => e.add_node(f, NEW_LABEL)?,
    };
    set_type(e, kits, std::slice::from_ref(&id), Some(ty))?;
    Ok(id)
}

/// The type's label and kit, as the panel's type row shows it.
fn type_row(kits: &Kits, ty: Option<&rhizo_core::TypeName>) -> Option<(String, Option<(String, String)>)> {
    let t = ty?;
    let def = kits.find_type(t.as_str());
    let kit = kits.owner(t.as_str()).map(|(k, _)| k.name.clone());
    Some((t.to_string(), def.map(|d| (d.label.clone(), kit.unwrap_or_default()))))
}

/// The panel for frame `f`: its type, layout, direction, colours and props
/// (`typing`: a prop being typed, with what it would become).
pub fn frame_model(
    doc: &Document,
    kits: &Kits,
    f: &FrameId,
    typing: Option<(&str, Result<Option<Value>, String>)>,
) -> Option<Model> {
    let frame = doc.frame(f)?;
    let def = frame.ty.as_ref().and_then(|t| kits.find_type(t.as_str()));
    let inside = doc.nodes.values().filter(|n| &n.frame == f).count();
    Some(Model {
        subject: Subject::Frame,
        label: frame.label.clone().unwrap_or_default(),
        id: format!("frame {f} · {}", crate::chrome::plural(inside, "node")),
        ty: type_row(kits, frame.ty.as_ref()),
        fields: relevant(inspector::fields(def, &frame.props, typing), environment_cloud(frame)),
        place: Place::Hidden,
        editable: true,
        settings: frame_settings(frame),
        add_typed: frame.ty.is_some(),
        ..Model::default()
    })
}

/// The panel for edge `k`: label, kind, line, arrowheads, colour,
/// predicate, and the props its kits give an edge between those types.
pub fn edge_model(doc: &Document, k: usize, typing: Option<(&str, Result<Option<Value>, String>)>) -> Option<Model> {
    let e = doc.edges.get(k)?;
    let ty = |id: &NodeId| doc.node(id).and_then(|n| n.ty.as_ref()).map(|t| t.as_str());
    let name = |id: &NodeId| doc.node(id).map_or_else(|| id.to_string(), |n| n.label.clone());
    let schema = rhizo_kit::edge_schema(ty(&e.from), ty(&e.to));
    Some(Model {
        subject: Subject::Edge,
        label: format!("{} to {}", name(&e.from), name(&e.to)),
        id: format!("edge {} {}", e.from, e.to),
        fields: inspector::fields_from_schema(schema.as_ref(), &|_, _| None, &e.props, typing),
        place: Place::Hidden,
        editable: true,
        settings: edge_settings(e),
        ..Model::default()
    })
}

/// The panel with nothing selected: the document, in file `file`.
pub fn document_model(doc: &Document, file: &str) -> Model {
    Model {
        subject: Subject::Document,
        label: doc.title.clone().unwrap_or_else(|| file.to_string()),
        id: format!("Document · {file}"),
        place: Place::Hidden,
        editable: true,
        settings: document_settings(doc),
        ..Model::default()
    }
}

/// The cloud of an infra environment frame (`aws` unless it says).
fn environment_cloud(f: &rhizo_core::Frame) -> Option<&str> {
    let env = f.ty.as_ref().is_some_and(|t| t.as_str() == rhizo_kit::infra::ENVIRONMENT);
    env.then(|| rhizo_kit::infra::cloud(f))
}

/// Fields without other clouds' settings blocks and `lock-file` (imported
/// code's), unless they are set: in a designed aws environment the azure and
/// gcp rows are only clutter.
fn relevant(fields: Vec<inspector::Field>, cloud: Option<&str>) -> Vec<inspector::Field> {
    let Some(cloud) = cloud else { return fields };
    fields
        .into_iter()
        .filter(|f| {
            let other_cloud = f
                .within
                .as_ref()
                .is_some_and(|(block, _)| ["aws", "azure", "gcp"].contains(&block.as_str()) && block != cloud);
            f.value.is_some() || !(other_cloud || f.key == "lock-file")
        })
        .collect()
}

/// The panel's look, type and criteria for one node (`editable` false:
/// a reference, shown not edited); the viewer adds where it is used.
pub fn node_model(
    doc: &Document,
    kits: &Kits,
    id: &NodeId,
    editable: bool,
    typing: Option<(&str, Result<Option<Value>, String>)>,
) -> Option<Model> {
    let node = doc.node(id)?;
    let def = node.ty.as_ref().and_then(|t| kits.find_type(t.as_str()));
    Some(Model {
        subject: Subject::Node,
        label: node.label.clone(),
        id: id.to_string(),
        ty: type_row(kits, node.ty.as_ref()),
        fields: relevant(
            inspector::fields(def, &node.props, typing),
            doc.frame(&node.frame).and_then(environment_cloud),
        ),
        editable,
        settings: if editable { node_settings(node) } else { Vec::new() },
        accept: editable.then(|| node.accept.clone()),
        ..Model::default()
    })
}

/// Names a frame just made ("New frame" until typed over): the label, and
/// an id that follows it, as for a frame written by hand. Returns the id.
pub fn name_new_frame(e: &mut Editor, frame: &FrameId, label: &str) -> Result<FrameId, EditError> {
    let nth = |e: &Editor, f: &FrameId| e.document().frames.iter().filter(|x| !x.implicit).position(|x| &x.id == f);
    let at = nth(e, frame).ok_or_else(|| EditError::UnknownFrame(frame.clone()))?;
    e.rename_frame(frame, label)?;
    e.set_frame_look(frame, "id", None)?;
    let id = e.document().frames.iter().filter(|x| !x.implicit).nth(at).map(|f| f.id.clone());
    id.ok_or_else(|| EditError::UnknownFrame(frame.clone()))
}

/// What [`Subject`] an owner is shown as.
pub fn subject(owner: &Owner) -> Subject {
    match owner {
        Owner::Nodes(_) => Subject::Node,
        Owner::Frame(_) => Subject::Frame,
        Owner::Edge(_) => Subject::Edge,
        Owner::Document => Subject::Document,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn an_environment_shows_only_its_own_clouds_settings() {
        let keys = |src: &str| {
            let loaded = rhizo_lang::load(src, rhizo_lang::Syntax::Kdl);
            let id = loaded.document.frames[0].id.clone();
            frame_model(&loaded.document, &Kits::builtin(), &id, None)
                .unwrap()
                .fields
                .into_iter()
                .map(|f| f.key)
                .collect::<Vec<_>>()
        };
        let aws = keys(
            "rhizo version=2\nuse infra version=\"0.1\"\nframe \"P\" type=infra.environment {\n    props cloud=\"aws\" region=\"eu-central-1\"\n}\n",
        );
        assert!(!aws.iter().any(|k| k.starts_with("azure.") || k.starts_with("gcp.") || k == "lock-file"), "{aws:?}");
        let azure = keys(
            "rhizo version=2\nuse infra version=\"0.1\"\nframe \"P\" type=infra.environment {\n    props cloud=\"azure\" region=\"westeurope\" {\n        gcp state-prefix=\"x\"\n    }\n}\n",
        );
        assert!(azure.contains(&"azure.resource-group".to_string()), "{azure:?}");
        assert!(
            azure.contains(&"gcp.state-prefix".to_string()),
            "a set value of another cloud stays visible: {azure:?}"
        );
        assert!(!azure.contains(&"gcp.state-bucket".to_string()), "{azure:?}");
    }

    #[test]
    fn settings_write_where_the_file_keeps_them() {
        let mut e = Editor::open(
            "rhizo version=2\nframe \"F\" id=f {\n    node \"A\" id=a\n    node \"B\" id=b\n    edge a b\n}\n",
        );
        let a = Owner::Nodes(vec!["a".into()]);
        apply_setting(&mut e, &a, SHAPE, Some("diamond")).unwrap();
        apply_setting(&mut e, &a, FILL, Some(&hex(FILLS[1].unwrap()))).unwrap();
        apply_setting(&mut e, &a, TAGS, Some("api, public")).unwrap();
        assert!(
            e.source().contains("node \"A\" id=a shape=diamond fill=\"#fee2e2\" tags=\"api public\""),
            "{}",
            e.source()
        );
        apply_setting(&mut e, &a, FILL, None).unwrap();
        assert!(!e.source().contains("fill="));
        let f = Owner::Frame("f".into());
        apply_setting(&mut e, &f, LAYOUT, Some("layered")).unwrap();
        apply_setting(&mut e, &f, DIRECTION, Some("down")).unwrap();
        assert!(e.source().contains("frame \"F\" id=f layout=layered direction=down {"), "{}", e.source());
        apply_setting(&mut e, &f, LAYOUT, Some("auto")).unwrap();
        assert!(!e.source().contains("layout="), "auto is the default: not written");
        let edge = Owner::Edge(0);
        apply_setting(&mut e, &edge, KIND, Some("data")).unwrap();
        apply_setting(&mut e, &edge, LINE, Some("dashed")).unwrap();
        apply_setting(&mut e, &edge, ARROW, Some("both")).unwrap();
        assert!(e.source().contains("edge a b kind=data line=dashed arrow=both"), "{}", e.source());
        apply_setting(&mut e, &Owner::Document, TITLE, Some("Shop")).unwrap();
        assert!(e.source().starts_with("rhizo version=2\ntitle \"Shop\"\n"), "{}", e.source());
        let doc = e.document();
        assert_eq!(node_settings(&doc.nodes[&NodeId::from("a")])[0].value, "diamond");
        assert_eq!(edge_settings(&doc.edges[0])[1].value, "data");
        assert_eq!(document_settings(doc)[0].value, "Shop");
        assert!(apply_setting(&mut e, &Owner::Document, SHAPE, Some("x")).is_err());
    }

    #[test]
    fn a_prop_inside_an_object_prop_writes_the_object() {
        let kits = Kits::builtin();
        let mut e = Editor::open("frame \"Prod\"\n");
        let f = FrameId::from("prod");
        set_frame_type(&mut e, &kits, &f, Some("infra.environment")).unwrap();
        let owner = Owner::Frame(f.clone());
        let within = ("azure".to_string(), "resource-group".to_string());
        set_prop(&mut e, &owner, Some(&within), "azure.resource-group", Some(json!("rg-shop"))).unwrap();
        assert_eq!(e.document().frame(&f).unwrap().props["azure"], json!({ "resource-group": "rg-shop" }));
        set_prop(&mut e, &owner, Some(&within), "azure.resource-group", None).unwrap();
        assert!(e.document().frame(&f).unwrap().props.is_empty(), "{}", e.source());
        assert_eq!(
            frame_types(&kits).iter().map(|t| t.0).collect::<Vec<_>>(),
            ["data.api", "flow.workflow", "infra.environment", "rhizo.flow"]
        );
    }
}
