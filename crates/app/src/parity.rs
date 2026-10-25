//! UI parity with the file format: every construct a `.rhizo` file can
//! hold, and what in the viewer creates, edits and deletes it. The parser's
//! keyword lists ([`rhizo_lang::kdl`]) are checked against this table, so a
//! construct added to the format without a way to make it in the viewer
//! fails a test until it gets one here, or is listed in [`TEXT_ONLY`] with
//! the reason. `docs/design/ui-parity.md` holds [`markdown`]'s output.

use crate::inspector::Subject;
use crate::ops;
use crate::palette::{self, Action, Arrange};

/// A way the viewer reaches a construct.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ui {
    /// A command of the registry: the menu bar, the palette, the context menus.
    Command(Action),
    /// A row of the inspector's settings, by its name.
    Setting(Subject, &'static str),
    /// The inspector's type row.
    TypeRow(Subject),
    /// The inspector's props form, made from the kits' schemas.
    Props(Subject),
    /// The inspector's acceptance criteria list.
    Accept,
    /// A pointer gesture on the canvas.
    Gesture(&'static str),
    /// Written by the viewer when another edit needs it.
    Auto(&'static str),
}

pub struct Construct {
    /// How the table names it.
    pub what: &'static str,
    /// The keywords it is written with: `top.title`, `node.shape`, `frame.layout`, `edge.kind`…
    pub keys: &'static [&'static str],
    pub create: &'static [Ui],
    pub edit: &'static [Ui],
    pub delete: &'static [Ui],
}

use Subject::{Document as D, Edge as E, Frame as F, Node as N};
use Ui::{Accept, Auto, Command as C, Gesture, Props, Setting as S, TypeRow};

pub const CONSTRUCTS: &[Construct] = &[
    Construct {
        what: "Format version",
        keys: &["top.rhizo"],
        create: &[Auto("raised to 2 when a typed feature is set")],
        edit: &[Auto("raised to 2 when a typed feature is set")],
        delete: &[],
    },
    Construct {
        what: "Document title",
        keys: &["top.title"],
        create: &[S(D, ops::TITLE), C(Action::EditTitle)],
        edit: &[S(D, ops::TITLE), C(Action::EditTitle)],
        delete: &[S(D, ops::TITLE)],
    },
    Construct {
        what: "Document theme",
        keys: &["top.theme"],
        create: &[S(D, ops::THEME), C(Action::DocThemeMenu)],
        edit: &[S(D, ops::THEME), C(Action::DocThemeMenu)],
        delete: &[S(D, ops::THEME)],
    },
    Construct {
        what: "Layout and direction of nodes outside frames",
        keys: &["top.layout", "top.direction"],
        create: &[S(D, ops::LAYOUT), S(D, ops::DIRECTION), C(Action::LayoutMenu), C(Action::DirectionMenu)],
        edit: &[S(D, ops::LAYOUT), S(D, ops::DIRECTION), C(Action::LayoutMenu), C(Action::DirectionMenu)],
        delete: &[S(D, ops::LAYOUT), S(D, ops::DIRECTION)],
    },
    Construct {
        what: "Kits in use (`use`)",
        keys: &["top.use"],
        create: &[S(D, ops::KITS), C(Action::KitsMenu), Auto("added when a type of the kit is set")],
        edit: &[C(Action::KitsMenu)],
        delete: &[S(D, ops::KITS), C(Action::KitsMenu)],
    },
    Construct {
        what: "Imports of other files",
        keys: &["top.import"],
        create: &[C(Action::InsertReferenceMenu), Gesture("drag a sidebar thing onto the canvas")],
        edit: &[],
        delete: &[C(Action::RemoveUnusedImports), Auto("offered when the last reference is deleted")],
    },
    Construct {
        what: "Frame",
        keys: &["top.frame", "frame.frame"],
        create: &[C(Action::NewFrame), C(Action::GroupFrame)],
        edit: &[Gesture("drag nodes into and out of it")],
        delete: &[C(Action::Delete), C(Action::Ungroup)],
    },
    Construct {
        what: "Frame label",
        keys: &["frame.label"],
        create: &[C(Action::NewFrame), C(Action::Rename)],
        edit: &[C(Action::Rename), Gesture("double-click the frame's label")],
        delete: &[],
    },
    Construct {
        what: "Frame id",
        keys: &["frame.id"],
        create: &[Auto("pinned when a rename would change it (it names compile output)")],
        edit: &[],
        delete: &[],
    },
    Construct {
        what: "Frame type (environment, API, workflow…)",
        keys: &["frame.type"],
        create: &[TypeRow(F), C(Action::FrameTypeMenu)],
        edit: &[TypeRow(F), C(Action::FrameTypeMenu)],
        delete: &[TypeRow(F), C(Action::FrameTypeMenu)],
    },
    Construct {
        what: "Frame layout and direction",
        keys: &["frame.layout", "frame.direction"],
        create: &[S(F, ops::LAYOUT), S(F, ops::DIRECTION), C(Action::LayoutMenu), C(Action::DirectionMenu)],
        edit: &[S(F, ops::LAYOUT), S(F, ops::DIRECTION), C(Action::LayoutMenu), C(Action::DirectionMenu)],
        delete: &[S(F, ops::LAYOUT), S(F, ops::DIRECTION)],
    },
    Construct {
        what: "Frame props",
        keys: &["frame.props"],
        create: &[Props(F)],
        edit: &[Props(F)],
        delete: &[Props(F)],
    },
    Construct {
        what: "Frame colours",
        keys: &["frame.fill", "frame.stroke"],
        create: &[S(F, ops::FILL), S(F, ops::STROKE), C(Action::FillMenu), C(Action::StrokeMenu)],
        edit: &[S(F, ops::FILL), S(F, ops::STROKE), C(Action::FillMenu), C(Action::StrokeMenu)],
        delete: &[S(F, ops::FILL), S(F, ops::STROKE)],
    },
    Construct {
        what: "Frame position",
        keys: &["frame.x", "frame.y"],
        create: &[C(Action::NewFrame)],
        edit: &[],
        delete: &[],
    },
    Construct {
        what: "Node",
        keys: &["top.node", "top.-", "frame.node", "frame.-", "node.node", "node.-"],
        create: &[
            C(Action::AddChild),
            C(Action::AddSibling),
            C(Action::AddTypedMenu),
            C(Action::Paste),
            Auto("an edge to a new name"),
        ],
        edit: &[Gesture("drag onto another node (outlines) or into a frame"), C(Action::GroupFrame)],
        delete: &[C(Action::Delete), C(Action::Cut)],
    },
    Construct {
        what: "Node label",
        keys: &["node.label"],
        create: &[C(Action::AddChild), C(Action::AddSibling)],
        edit: &[C(Action::Rename), Gesture("double-click the node")],
        delete: &[],
    },
    Construct {
        what: "Node id",
        keys: &["node.id"],
        create: &[Auto("from the label typed for a new node; pinned on typed nodes and on renames")],
        edit: &[],
        delete: &[],
    },
    Construct {
        what: "Node type",
        keys: &["node.type"],
        create: &[TypeRow(N), C(Action::ChangeTypeMenu), C(Action::AddTypedMenu)],
        edit: &[TypeRow(N), C(Action::ChangeTypeMenu)],
        delete: &[TypeRow(N), C(Action::ChangeTypeMenu)],
    },
    Construct {
        what: "Node shape",
        keys: &["node.shape"],
        create: &[S(N, ops::SHAPE), C(Action::ShapeMenu)],
        edit: &[S(N, ops::SHAPE), C(Action::ShapeMenu)],
        delete: &[S(N, ops::SHAPE)],
    },
    Construct {
        what: "Node colours",
        keys: &["node.fill", "node.stroke"],
        create: &[S(N, ops::FILL), S(N, ops::STROKE), C(Action::FillMenu), C(Action::StrokeMenu)],
        edit: &[S(N, ops::FILL), S(N, ops::STROKE), C(Action::FillMenu), C(Action::StrokeMenu)],
        delete: &[S(N, ops::FILL), S(N, ops::STROKE)],
    },
    Construct {
        what: "Node note, link, tags and classes",
        keys: &["node.note", "node.link", "node.tags", "node.class"],
        create: &[S(N, ops::NOTE), S(N, ops::LINK), S(N, ops::TAGS), S(N, ops::CLASS)],
        edit: &[S(N, ops::NOTE), S(N, ops::LINK), S(N, ops::TAGS), S(N, ops::CLASS)],
        delete: &[S(N, ops::NOTE), S(N, ops::LINK), S(N, ops::TAGS), S(N, ops::CLASS)],
    },
    Construct {
        what: "Node position (free layouts)",
        keys: &["node.x", "node.y"],
        create: &[Gesture("drag in a free frame"), C(Action::Arrange(Arrange::Left))],
        edit: &[Gesture("drag in a free frame"), C(Action::Arrange(Arrange::SpreadX))],
        delete: &[Auto("dropped when the node moves to another frame")],
    },
    Construct {
        what: "Node props",
        keys: &["node.props"],
        create: &[Props(N)],
        edit: &[Props(N)],
        delete: &[Props(N)],
    },
    Construct {
        what: "Acceptance criteria",
        keys: &["node.accept"],
        create: &[Accept, C(Action::AddAccept)],
        edit: &[Accept],
        delete: &[Accept],
    },
    Construct {
        what: "Edge",
        keys: &["top.edge", "frame.edge"],
        create: &[C(Action::Connect), Gesture("⌥-drag from node to node (any drag in flowcharts)")],
        edit: &[],
        delete: &[C(Action::Delete)],
    },
    Construct {
        what: "Edge label",
        keys: &["edge.label"],
        create: &[S(E, ops::LABEL), C(Action::Rename)],
        edit: &[S(E, ops::LABEL), C(Action::Rename), Gesture("double-click the edge")],
        delete: &[S(E, ops::LABEL), C(Action::Rename)],
    },
    Construct {
        what: "Edge kind",
        keys: &["edge.kind"],
        create: &[S(E, ops::KIND), C(Action::EdgeKindMenu)],
        edit: &[S(E, ops::KIND), C(Action::EdgeKindMenu)],
        delete: &[S(E, ops::KIND), C(Action::EdgeKindMenu)],
    },
    Construct {
        what: "Edge line style",
        keys: &["edge.line"],
        create: &[S(E, ops::LINE), C(Action::LineMenu)],
        edit: &[S(E, ops::LINE), C(Action::LineMenu)],
        delete: &[S(E, ops::LINE), C(Action::LineMenu)],
    },
    Construct {
        what: "Edge arrowheads",
        keys: &["edge.arrow"],
        create: &[S(E, ops::ARROW), C(Action::ArrowMenu)],
        edit: &[S(E, ops::ARROW), C(Action::ArrowMenu)],
        delete: &[S(E, ops::ARROW), C(Action::ArrowMenu)],
    },
    Construct {
        what: "Edge colour",
        keys: &["edge.stroke"],
        create: &[S(E, ops::STROKE)],
        edit: &[S(E, ops::STROKE)],
        delete: &[S(E, ops::STROKE)],
    },
    Construct {
        what: "Edge predicate (`type`)",
        keys: &["edge.type"],
        create: &[S(E, ops::PREDICATE)],
        edit: &[S(E, ops::PREDICATE)],
        delete: &[S(E, ops::PREDICATE)],
    },
    Construct {
        what: "Edge props (`access`, `prefix`, `route`, `cardinality`)",
        keys: &["edge.props"],
        create: &[Props(E)],
        edit: &[Props(E)],
        delete: &[Props(E)],
    },
];

/// Written in the text only (the text pane, ⌘\, edits them), and why.
pub const TEXT_ONLY: &[(&str, &str)] = &[
    ("top.base", "base IRI for linked-data ids: rare, and set once"),
    ("top.prefix", "CURIE prefixes for linked-data ids: rare, and set once"),
    ("top.style", "named styles (classes): a small stylesheet, clearer as text"),
    ("node.port", "ports refine a kit type's own; typed nodes get theirs from the kit"),
    ("node.prov", "provenance is written by importers and agents, and reviewed in proposals"),
    ("node.text", "text colour: chosen for contrast with the fill"),
    ("node.stroke-width", "fine typography and line weights"),
    ("node.font-size", "fine typography and line weights"),
    ("node.font-weight", "fine typography and line weights"),
    ("node.line", "an outline's line style: rare on nodes"),
    ("edge.id", "edge ids are for statements that point at edges (linked data)"),
    ("edge.class", "classes need a `style`, which is text-only"),
    ("edge.from-port", "ports come from the kit types at both ends"),
    ("edge.to-port", "ports come from the kit types at both ends"),
    ("edge.prov", "provenance is written by importers and agents"),
    ("edge.fill", "edges have no fill"),
    ("edge.text", "label colour: chosen for contrast"),
    ("edge.stroke-width", "fine line weights"),
    ("edge.font-size", "fine typography"),
    ("edge.font-weight", "fine typography"),
    ("edge.shape", "edges have no shape"),
    ("frame.columns", "grid columns: the grid picks them from the content"),
    ("frame.text", "label colour: chosen for contrast"),
    ("frame.stroke-width", "fine line weights"),
    ("frame.font-size", "fine typography"),
    ("frame.font-weight", "fine typography"),
    ("frame.line", "a frame's line style: rare"),
    ("frame.shape", "frames are always rounded boxes"),
];

/// Every keyword the parser reads, as `top.title`, `node.shape`, `edge.kind`, `frame.layout`.
pub fn keywords() -> Vec<String> {
    use rhizo_lang::kdl::*;
    let with = |scope: &str, lists: &[&[&str]]| -> Vec<String> {
        lists.iter().flat_map(|l| l.iter()).map(|k| format!("{scope}.{k}")).collect()
    };
    let mut out = with("top", &[TOP_LEVEL]);
    out.extend(with("node", &[NODE_PROPS, STYLE_PROPS, NODE_CHILDREN]));
    out.extend(with("edge", &[EDGE_PROPS, STYLE_PROPS, EDGE_CHILDREN]));
    out.extend(with("frame", &[FRAME_PROPS, STYLE_PROPS, FRAME_CHILDREN]));
    out.sort();
    out.dedup();
    out
}

/// What a construct's commands act on, so Delete reads "Delete frame…" for frames.
fn selection_of(c: &Construct) -> palette::Selection {
    match c.what.split(' ').next() {
        Some("Frame") => palette::Selection::Frame,
        Some("Edge") => palette::Selection::Edge,
        Some("Node" | "Acceptance") => palette::Selection::Node,
        _ => palette::Selection::Nothing,
    }
}

fn ui_text(u: &Ui, on: palette::Selection) -> String {
    let subject = |s: &Subject| match s {
        Subject::Node => "node",
        Subject::Frame => "frame",
        Subject::Edge => "edge",
        Subject::Document => "document",
    };
    match u {
        Ui::Command(a) => {
            let fits = palette::COMMANDS.iter().find(|c| c.action == *a && c.needs.met_by(on));
            let c = fits.or_else(|| palette::command(*a)).expect("a command of the registry");
            if c.keys.is_empty() { format!("“{}”", c.name) } else { format!("“{}” ({})", c.name, c.keys) }
        }
        Ui::Setting(s, name) => format!("inspector: {} › {name}", subject(s)),
        Ui::TypeRow(s) => format!("inspector: {} › Type", subject(s)),
        Ui::Props(s) => format!("inspector: {} › Props", subject(s)),
        Ui::Accept => "inspector: node › Acceptance criteria".into(),
        Ui::Gesture(g) => g.to_string(),
        Ui::Auto(a) => format!("automatic: {a}"),
    }
}

/// The parity table, as `docs/design/ui-parity.md` shows it.
pub fn markdown() -> String {
    let cell = |us: &[Ui], on| {
        if us.is_empty() { "—".to_string() } else { us.iter().map(|u| ui_text(u, on)).collect::<Vec<_>>().join("; ") }
    };
    let mut out = String::from("| Construct | Written as | Create | Edit | Delete |\n|---|---|---|---|---|\n");
    for c in CONSTRUCTS {
        let keys: Vec<String> = c.keys.iter().map(|k| format!("`{k}`")).collect();
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} |\n",
            c.what,
            keys.join(" "),
            cell(c.create, selection_of(c)),
            cell(c.edit, selection_of(c)),
            cell(c.delete, selection_of(c))
        ));
    }
    out.push_str("\nText only (the text pane, ⌘\\, edits them):\n\n| Written as | Why |\n|---|---|\n");
    for (k, why) in TEXT_ONLY {
        out.push_str(&format!("| `{k}` | {why} |\n"));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::inspector::SettingKind;

    /// The settings each subject shows, from a document that has all of them.
    fn settings(s: Subject) -> Vec<String> {
        let src = "rhizo version=2\n- \"Loose\"\nframe \"F\" id=f {\n    node \"A\" id=a\n    node \"B\" id=b\n    edge a b\n}\n";
        let e = rhizo_lang::Editor::open(src);
        let doc = e.document();
        let rows = match s {
            Subject::Node => ops::node_settings(&doc.nodes[&rhizo_core::NodeId::from("a")]),
            Subject::Frame => ops::frame_settings(doc.frame(&"f".into()).unwrap()),
            Subject::Edge => ops::edge_settings(&doc.edges[0]),
            Subject::Document => ops::document_settings(doc),
        };
        rows.into_iter().filter(|r| r.kind != SettingKind::Info).map(|r| r.name).collect()
    }

    #[test]
    fn every_construct_of_the_format_has_a_way_in_the_viewer() {
        let covered: Vec<&str> = CONSTRUCTS.iter().flat_map(|c| c.keys.iter().copied()).collect();
        let text_only: Vec<&str> = TEXT_ONLY.iter().map(|(k, _)| *k).collect();
        for k in keywords() {
            let ui = covered.contains(&k.as_str());
            let text = text_only.contains(&k.as_str());
            assert!(ui || text, "`{k}` can be written but neither the viewer nor TEXT_ONLY knows it");
            assert!(!(ui && text), "`{k}` is both in the table and text-only");
        }
        for k in covered.iter().chain(&text_only) {
            assert!(keywords().iter().any(|w| w == k), "`{k}` is not a keyword the parser reads");
        }
        for c in CONSTRUCTS {
            assert!(!c.create.is_empty(), "{}: nothing creates it", c.what);
            for u in c.create.iter().chain(c.edit).chain(c.delete) {
                match u {
                    Ui::Command(a) => assert!(palette::command(*a).is_some(), "{a:?} is not in the registry"),
                    Ui::Setting(s, name) => {
                        assert!(settings(*s).iter().any(|n| n == name), "{s:?} shows no setting {name}")
                    }
                    _ => {}
                }
            }
        }
    }

    #[test]
    fn the_design_doc_holds_the_table() {
        let doc = include_str!("../../../docs/design/ui-parity.md");
        assert!(
            doc.contains(&markdown()),
            "update the table in docs/design/ui-parity.md: \
             cargo test -p rhizo-app --lib parity::tests::print_the_table -- --ignored --nocapture"
        );
    }

    #[test]
    #[ignore = "prints the table for the design doc"]
    fn print_the_table() {
        print!("{}", markdown());
    }
}
