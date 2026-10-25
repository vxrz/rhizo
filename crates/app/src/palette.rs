//! The ⌘K command palette: every viewer action, filterable by typing. It
//! also lists templates (⌘N), recent files (⇧⌘O), the files of the project
//! (⌘P), its problems (⇧⌘M), things to refer to (⌘R) and where a node is
//! used (⇧⌘U).

use std::path::PathBuf;

use rhizo_core::Color;
use rhizo_render::Fonts;
use vello::Scene;
use vello::kurbo::{Affine, Line, Point, Rect, RoundedRect, Stroke, Vec2};
use vello::peniko::Fill;

use crate::chrome::{self, ACCENT};
use crate::draw::{self, FontCache};
use crate::field::{self, Field};
use crate::sidebar::{self, Glyph};
use crate::{files, peniko_color};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    AddChild,
    AddSibling,
    Rename,
    Delete,
    Connect,
    Undo,
    Redo,
    Fit,
    ZoomActual,
    /// Lays the diagram out from scratch, as exports do.
    TidyLayout,
    ZoomIn,
    ZoomOut,
    /// Lists the templates.
    NewMenu,
    /// A new file from the starter of this kind.
    New(&'static str),
    Open,
    /// A folder as a project.
    OpenFolder,
    /// Lists the recent files.
    RecentMenu,
    /// Lists the files of the project.
    GoToFile,
    /// An entry of the list shown (a recent file, a problem…), by index.
    Pick(usize),
    /// Shows or hides the project sidebar.
    ToggleSidebar,
    /// Lists the project's problems.
    ProblemsMenu,
    /// Lists what the open file can refer to in other files.
    InsertReferenceMenu,
    /// Lists where the selected node is used in other files.
    FindUsages,
    /// Opens what the selected reference points at.
    GoToDefinition,
    /// Shows or hides the inspector.
    ToggleInspector,
    /// Lists node types, then adds a node of the one chosen.
    AddTypedMenu,
    /// Lists the types the selected node can take.
    ChangeTypeMenu,
    /// Back to the file before the last jump.
    Back,
    SaveAs,
    Export,
    Close,
    Reveal,
    OpenInEditor,
    /// Shows or hides the text pane.
    TogglePane,
    /// Writes the pane's text now, even with errors.
    SaveText,
    /// Drops unsaved typing for the file as it is on disk.
    RevertText,
    /// Switches the selected node of imported code to rhizo's code (a dry
    /// run in the background, then a report to confirm).
    AdoptNode,
    /// Adopts every node of imported code that switches without a change.
    AdoptAll,
    /// Stops a running adopt, or drops its waiting report.
    CancelAdopt,
    /// The selected nodes, or the focused text's selection.
    Cut,
    Copy,
    /// Nodes copied as rhizo text, or text into the focused field.
    Paste,
    SelectAll,
    /// Lists the open file's nodes, to select one.
    FindNode,
    /// Lines up or spreads out the selected nodes of a free frame.
    Arrange(Arrange),
    /// Draws the diagram in this theme.
    Theme(&'static str),
    /// Writes the specs the document compiles to into a folder.
    Compile,
    /// Opens the build brief an agent works from.
    Brief,
    /// Lists every command with its keys.
    Shortcuts,
    /// Opens the guide to writing rhizo.
    Guide,
    /// The ⌘K palette itself (for the menu bar and the key table).
    Palette,
    Quit,
    /// The About dialog: version, licence, where it is made.
    About,
    /// An empty frame: inside the selected frame, at the right-clicked
    /// point, or at the end of the file.
    NewFrame,
    /// Moves the selected nodes into a new frame.
    GroupFrame,
    /// Removes the selected frame, keeping what it holds.
    Ungroup,
    /// Lists the frame types (environment, API, workflow…) for the selected frame.
    FrameTypeMenu,
    /// Lists the layouts, for the selected frame (or the selection's frame).
    LayoutMenu,
    /// Lists the directions, likewise.
    DirectionMenu,
    /// Lists node shapes, for the selected nodes.
    ShapeMenu,
    /// Lists fill colours, for the selected nodes or frame.
    FillMenu,
    /// Lists stroke colours, for the selected nodes or frame.
    StrokeMenu,
    /// Adds an acceptance criterion (`accept`) to the selected node.
    AddAccept,
    /// Lists edge kinds (data, control, dependency), for the selected edge.
    EdgeKindMenu,
    /// Lists line styles, for the selected edge.
    LineMenu,
    /// Lists arrowheads, for the selected edge.
    ArrowMenu,
    /// Types the document's title in the inspector.
    EditTitle,
    /// Lists the kits, to use one or stop using it.
    KitsMenu,
    /// Lists the themes the document can name (`theme`).
    DocThemeMenu,
    /// Removes `import`s no node refers to any more.
    RemoveUnusedImports,
    /// A copy of the tutorial, saved where a dialog says.
    Learn,
    /// The gallery of samples, to open a copy of one.
    Gallery,
    /// Shows or hides the status bar's hints for what is selected.
    ToggleHints,
}

/// How [`Action::Arrange`] places the selected nodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrange {
    Left,
    Center,
    Right,
    Top,
    Middle,
    Bottom,
    /// Even gaps from left to right.
    SpreadX,
    /// Even gaps from top to bottom.
    SpreadY,
}

/// What is open and selected.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Selection {
    /// The welcome screen.
    NoDocument,
    Nothing,
    Node,
    Edge,
    /// Two nodes or more.
    Nodes,
    /// An explicit frame.
    Frame,
}

/// What a command needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Needs {
    Always,
    Document,
    /// Exactly one node.
    Node,
    Edge,
    /// One node or more.
    AnyNodes,
    /// Two nodes or more.
    Many,
    Frame,
    /// Nodes or a frame: what has a fill and a stroke.
    Boxes,
}

impl Needs {
    pub fn met_by(self, s: Selection) -> bool {
        match self {
            Needs::Always => true,
            Needs::Document => s != Selection::NoDocument,
            Needs::Node => s == Selection::Node,
            Needs::Edge => s == Selection::Edge,
            Needs::AnyNodes => matches!(s, Selection::Node | Selection::Nodes),
            Needs::Many => s == Selection::Nodes,
            Needs::Frame => s == Selection::Frame,
            Needs::Boxes => matches!(s, Selection::Node | Selection::Nodes | Selection::Frame),
        }
    }
}

/// One entry of the command registry: the palette, the menu bar, context
/// menus, tooltips and the key table all read it, so a shortcut is written
/// once, in `keys`.
pub struct Command {
    pub name: &'static str,
    /// As shown: `⇧⌘Z`, `Tab`, `⌥ drag`. Those with ⌘ are also the key
    /// table's chords ([`chord`]).
    pub keys: &'static str,
    /// Delete and Rename act on whatever is selected, node or edge.
    pub action: Action,
    pub needs: Needs,
    /// Other words that find it in the palette, space separated: what a
    /// person might call it ("cloud", "aws" for a frame type).
    pub words: &'static str,
}

pub const REVEAL: &str = if cfg!(target_os = "macos") {
    "Reveal in Finder"
} else if cfg!(windows) {
    "Show in Explorer"
} else {
    "Show in folder"
};

pub const COMMANDS: &[Command] = &[
    Command { name: "Add child", keys: "Tab", action: Action::AddChild, needs: Needs::Node, words: "new node add tab" },
    Command {
        name: "Add sibling",
        keys: "Enter",
        action: Action::AddSibling,
        needs: Needs::Node,
        words: "new node add",
    },
    Command { name: "Rename", keys: "F2", action: Action::Rename, needs: Needs::Node, words: "label edit title text" },
    Command { name: "Delete", keys: "⌫", action: Action::Delete, needs: Needs::AnyNodes, words: "remove erase" },
    Command { name: "Cut", keys: "⌘X", action: Action::Cut, needs: Needs::AnyNodes, words: "" },
    Command { name: "Copy", keys: "⌘C", action: Action::Copy, needs: Needs::AnyNodes, words: "" },
    Command { name: "Paste", keys: "⌘V", action: Action::Paste, needs: Needs::Document, words: "" },
    Command { name: "Select all", keys: "⌘A", action: Action::SelectAll, needs: Needs::Document, words: "" },
    Command {
        name: "Find node…",
        keys: "⌘F",
        action: Action::FindNode,
        needs: Needs::Document,
        words: "search jump go label",
    },
    Command {
        name: "Align left edges",
        keys: "",
        action: Action::Arrange(Arrange::Left),
        needs: Needs::Many,
        words: "",
    },
    Command {
        name: "Align centres",
        keys: "",
        action: Action::Arrange(Arrange::Center),
        needs: Needs::Many,
        words: "",
    },
    Command {
        name: "Align right edges",
        keys: "",
        action: Action::Arrange(Arrange::Right),
        needs: Needs::Many,
        words: "",
    },
    Command { name: "Align top edges", keys: "", action: Action::Arrange(Arrange::Top), needs: Needs::Many, words: "" },
    Command {
        name: "Align middles",
        keys: "",
        action: Action::Arrange(Arrange::Middle),
        needs: Needs::Many,
        words: "",
    },
    Command {
        name: "Align bottom edges",
        keys: "",
        action: Action::Arrange(Arrange::Bottom),
        needs: Needs::Many,
        words: "",
    },
    Command {
        name: "Distribute horizontally",
        keys: "",
        action: Action::Arrange(Arrange::SpreadX),
        needs: Needs::Many,
        words: "",
    },
    Command {
        name: "Distribute vertically",
        keys: "",
        action: Action::Arrange(Arrange::SpreadY),
        needs: Needs::Many,
        words: "",
    },
    Command {
        name: "Add typed node…",
        keys: "",
        action: Action::AddTypedMenu,
        needs: Needs::Document,
        words: "kit type typed bucket function queue database entity",
    },
    Command {
        name: "Change type…",
        keys: "",
        action: Action::ChangeTypeMenu,
        needs: Needs::AnyNodes,
        words: "kit type typed",
    },
    Command {
        name: "Adopt node…",
        keys: "",
        action: Action::AdoptNode,
        needs: Needs::Node,
        words: "terraform tofu import",
    },
    Command {
        name: "Adopt all that switch without changes",
        keys: "",
        action: Action::AdoptAll,
        needs: Needs::Document,
        words: "",
    },
    Command { name: "Cancel adopt", keys: "", action: Action::CancelAdopt, needs: Needs::Document, words: "" },
    Command {
        name: "Toggle inspector",
        keys: "⌘I",
        action: Action::ToggleInspector,
        needs: Needs::Document,
        words: "properties props details panel type",
    },
    Command {
        name: "Connect to…",
        keys: "⌥ drag",
        action: Action::Connect,
        needs: Needs::Node,
        words: "link arrow edge line relate",
    },
    Command {
        name: "Delete edge",
        keys: "⌫",
        action: Action::Delete,
        needs: Needs::Edge,
        words: "link arrow connection remove",
    },
    Command {
        name: "Edit edge label",
        keys: "F2",
        action: Action::Rename,
        needs: Needs::Edge,
        words: "link arrow text",
    },
    Command {
        name: "Set edge kind…",
        keys: "",
        action: Action::EdgeKindMenu,
        needs: Needs::Edge,
        words: "link arrow connect data control dependency",
    },
    Command {
        name: "Set line style…",
        keys: "",
        action: Action::LineMenu,
        needs: Needs::Edge,
        words: "edge link dashed dotted",
    },
    Command {
        name: "Set arrowheads…",
        keys: "",
        action: Action::ArrowMenu,
        needs: Needs::Edge,
        words: "edge link arrow",
    },
    Command {
        name: "New frame",
        keys: "",
        action: Action::NewFrame,
        needs: Needs::Document,
        words: "environment cloud aws azure gcp box container group",
    },
    Command {
        name: "Group into frame",
        keys: "⌘G",
        action: Action::GroupFrame,
        needs: Needs::AnyNodes,
        words: "group box container wrap",
    },
    Command { name: "Rename frame", keys: "F2", action: Action::Rename, needs: Needs::Frame, words: "" },
    Command {
        name: "Set frame type…",
        keys: "",
        action: Action::FrameTypeMenu,
        needs: Needs::Frame,
        words: "environment cloud aws azure gcp kind type workflow api",
    },
    Command { name: "Set layout…", keys: "", action: Action::LayoutMenu, needs: Needs::Document, words: "" },
    Command { name: "Set direction…", keys: "", action: Action::DirectionMenu, needs: Needs::Document, words: "" },
    Command { name: "Ungroup frame", keys: "", action: Action::Ungroup, needs: Needs::Frame, words: "" },
    Command { name: "Delete frame…", keys: "⌫", action: Action::Delete, needs: Needs::Frame, words: "" },
    Command {
        name: "Set shape…",
        keys: "",
        action: Action::ShapeMenu,
        needs: Needs::AnyNodes,
        words: "look style circle pill diamond",
    },
    Command {
        name: "Set fill colour…",
        keys: "",
        action: Action::FillMenu,
        needs: Needs::Boxes,
        words: "color look style background",
    },
    Command {
        name: "Set stroke colour…",
        keys: "",
        action: Action::StrokeMenu,
        needs: Needs::Boxes,
        words: "color look style border outline",
    },
    Command { name: "Add acceptance criterion", keys: "", action: Action::AddAccept, needs: Needs::Node, words: "" },
    Command { name: "Edit document title", keys: "", action: Action::EditTitle, needs: Needs::Document, words: "" },
    Command {
        name: "Kits in use…",
        keys: "",
        action: Action::KitsMenu,
        needs: Needs::Document,
        words: "kit type typed",
    },
    Command { name: "Document theme…", keys: "", action: Action::DocThemeMenu, needs: Needs::Document, words: "" },
    Command {
        name: "Remove unused imports",
        keys: "",
        action: Action::RemoveUnusedImports,
        needs: Needs::Document,
        words: "",
    },
    Command { name: "Undo", keys: "⌘Z", action: Action::Undo, needs: Needs::Document, words: "revert oops" },
    Command { name: "Redo", keys: "⇧⌘Z", action: Action::Redo, needs: Needs::Document, words: "again" },
    Command { name: "Fit to window", keys: "F", action: Action::Fit, needs: Needs::Document, words: "" },
    Command {
        name: "Tidy layout",
        keys: "",
        action: Action::TidyLayout,
        needs: Needs::Document,
        words: "arrange auto clean relayout",
    },
    Command { name: "Zoom to 100%", keys: "0", action: Action::ZoomActual, needs: Needs::Document, words: "" },
    Command { name: "Zoom in", keys: "+", action: Action::ZoomIn, needs: Needs::Document, words: "" },
    Command { name: "Zoom out", keys: "−", action: Action::ZoomOut, needs: Needs::Document, words: "" },
    Command {
        name: "New from template…",
        keys: "⌘N",
        action: Action::NewMenu,
        needs: Needs::Always,
        words: "starter create",
    },
    Command {
        name: "Open…",
        keys: "⌘O",
        action: Action::Open,
        needs: Needs::Always,
        words: "import mermaid markdown file",
    },
    Command { name: "Open folder…", keys: "⌥⌘O", action: Action::OpenFolder, needs: Needs::Always, words: "" },
    Command { name: "Open recent…", keys: "⇧⌘O", action: Action::RecentMenu, needs: Needs::Always, words: "" },
    Command {
        name: "Go to file in project…", keys: "⌘P", action: Action::GoToFile, needs: Needs::Document, words: ""
    },
    Command { name: "Toggle sidebar", keys: "⌘1", action: Action::ToggleSidebar, needs: Needs::Document, words: "" },
    Command {
        name: "Show problems in the project",
        keys: "⇧⌘M",
        action: Action::ProblemsMenu,
        needs: Needs::Document,
        words: "errors warnings check",
    },
    Command {
        name: "Insert reference…",
        keys: "⌘R",
        action: Action::InsertReferenceMenu,
        needs: Needs::Document,
        words: "",
    },
    Command { name: "Find usages", keys: "⇧⌘U", action: Action::FindUsages, needs: Needs::Node, words: "" },
    Command {
        name: "Go to definition",
        keys: "⌘ click",
        action: Action::GoToDefinition,
        needs: Needs::Node,
        words: "",
    },
    Command { name: "Back", keys: "⌘[", action: Action::Back, needs: Needs::Document, words: "" },
    Command { name: "Save as…", keys: "⇧⌘S", action: Action::SaveAs, needs: Needs::Document, words: "" },
    Command {
        name: "Export…",
        keys: "⌘E",
        action: Action::Export,
        needs: Needs::Document,
        words: "png svg pdf image picture mermaid d2 dot drawio save",
    },
    Command {
        name: "Toggle text pane",
        keys: "⌘\\",
        action: Action::TogglePane,
        needs: Needs::Document,
        words: "code kdl source text editor",
    },
    Command { name: "Save text now", keys: "⌘S", action: Action::SaveText, needs: Needs::Document, words: "" },
    Command {
        name: "Revert text to the file on disk",
        keys: "",
        action: Action::RevertText,
        needs: Needs::Document,
        words: "",
    },
    Command { name: "Open in text editor", keys: "", action: Action::OpenInEditor, needs: Needs::Document, words: "" },
    Command { name: REVEAL, keys: "", action: Action::Reveal, needs: Needs::Document, words: "" },
    Command { name: "Close file", keys: "⌘W", action: Action::Close, needs: Needs::Document, words: "" },
    Command { name: "Light theme", keys: "", action: Action::Theme("light"), needs: Needs::Always, words: "" },
    Command { name: "Dark theme", keys: "", action: Action::Theme("dark"), needs: Needs::Always, words: "" },
    Command { name: "Paper theme", keys: "", action: Action::Theme("paper"), needs: Needs::Always, words: "" },
    Command {
        name: "Compile specs…",
        keys: "",
        action: Action::Compile,
        needs: Needs::Document,
        words: "deploy terraform tofu opentofu openapi asyncapi spec schema generate build",
    },
    Command { name: "Build brief", keys: "", action: Action::Brief, needs: Needs::Document, words: "agent ai tasks" },
    Command {
        name: "Keyboard shortcuts",
        keys: "⌘/",
        action: Action::Shortcuts,
        needs: Needs::Always,
        words: "keys help hotkeys",
    },
    Command {
        name: "Learn rhizo",
        keys: "",
        action: Action::Learn,
        needs: Needs::Always,
        words: "tutorial help start begin lesson",
    },
    Command {
        name: "Open sample…",
        keys: "",
        action: Action::Gallery,
        needs: Needs::Always,
        words: "gallery example demo",
    },
    Command {
        name: "Toggle status-bar hints",
        keys: "",
        action: Action::ToggleHints,
        needs: Needs::Always,
        words: "hide show tips help",
    },
    Command {
        name: "Guide to writing rhizo",
        keys: "",
        action: Action::Guide,
        needs: Needs::Always,
        words: "help docs syntax manual",
    },
    Command { name: "Command palette", keys: "⌘K", action: Action::Palette, needs: Needs::Always, words: "" },
    Command { name: "About rhizo", keys: "", action: Action::About, needs: Needs::Always, words: "" },
    Command { name: "Quit rhizo", keys: "⌘Q", action: Action::Quit, needs: Needs::Always, words: "" },
];

/// The registry's entry for `action`: the first, where the palette has a
/// node and an edge form (Delete, Rename).
pub fn command(action: Action) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.action == action)
}

/// The chord in `keys` when it holds ⌘: shift, alt and the key, lowercase.
pub fn chord(keys: &str) -> Option<(bool, bool, String)> {
    let key = keys.trim_start_matches(['⇧', '⌥', '⌘']);
    let mods = &keys[..keys.len() - key.len()];
    (mods.contains('⌘') && !key.is_empty() && !key.contains(' '))
        .then(|| (mods.contains('⇧'), mods.contains('⌥'), key.to_lowercase()))
}

/// What a hover over a command's control says: its name and keys.
pub fn tip(action: Action) -> String {
    match command(action) {
        Some(c) if c.keys.is_empty() => c.name.to_string(),
        Some(c) => format!("{}   {}", c.name, c.keys),
        None => String::new(),
    }
}

/// What the status bar suggests for the selection, or for the text pane
/// when it has the keys: a command's keys and a word on what they do (its
/// name when it has no keys), from the registry, at most four.
pub fn hints(selected: Selection, pane: bool) -> Vec<String> {
    use Action::*;
    let wanted: &[(Action, &str)] = match selected {
        _ if pane => &[(SaveText, "save"), (Undo, "undo"), (TogglePane, "hide text")],
        Selection::NoDocument => &[],
        Selection::Nothing => &[(AddChild, "add a node"), (FindNode, "find"), (Palette, "commands")],
        Selection::Node => {
            &[(AddChild, "child"), (AddSibling, "sibling"), (Rename, "rename"), (ToggleInspector, "inspect")]
        }
        Selection::Nodes => &[(GroupFrame, "group"), (Delete, "delete"), (Copy, "copy")],
        Selection::Frame => &[(FrameTypeMenu, ""), (AddTypedMenu, ""), (Rename, "rename"), (Delete, "delete")],
        Selection::Edge => &[(Rename, "label"), (Delete, "delete"), (EdgeKindMenu, ""), (LineMenu, "")],
    };
    wanted
        .iter()
        .filter_map(|&(a, word)| {
            let c = command(a)?;
            Some(if c.keys.is_empty() || word.is_empty() { c.name.to_string() } else { format!("{} {word}", c.keys) })
        })
        .take(4)
        .collect()
}

/// What the palette lists.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum List {
    Commands,
    Templates,
    Recent,
    Project,
    Problems,
    References,
    Usages,
    /// The types the selected node can take.
    Types,
    /// The types of a node to add.
    NewTyped,
    /// The values a prop can take.
    Values,
    /// Every command with its keys, whatever is selected.
    Shortcuts,
    /// The open file's nodes.
    Nodes,
}

impl List {
    /// Shown while nothing is typed.
    pub fn placeholder(self) -> &'static str {
        match self {
            List::Commands => "Type a command…",
            List::Templates => "New from template…",
            List::Recent => "Open recent…",
            List::Project => "Go to file in project…",
            List::Problems => "Problems in the project…",
            List::References => "Insert a reference to…",
            List::Usages => "Where it is used…",
            List::Types => "Set type…",
            List::NewTyped => "Add a node of type…",
            List::Values => "Choose a value…",
            List::Shortcuts => "Keyboard shortcuts…",
            List::Nodes => "Find node…",
        }
    }

    /// Typing also matches the grey detail: a file name finds its problems.
    fn searches_detail(self) -> bool {
        matches!(
            self,
            List::Problems
                | List::References
                | List::Usages
                | List::Types
                | List::NewTyped
                | List::Shortcuts
                | List::Nodes
        )
    }
}

/// An entry of a list the window fills: recent files, problems, things…
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub detail: String,
    pub glyph: Option<Glyph>,
}

/// Files as entries: name, and the folder in grey.
pub fn path_entries(paths: &[PathBuf]) -> Vec<Entry> {
    paths
        .iter()
        .map(|p| Entry {
            name: files::name(p),
            detail: p.parent().map(files::display_path).unwrap_or_default(),
            glyph: Some(if p.is_dir() { Glyph::Folder } else { Glyph::File }),
        })
        .collect()
}

/// One line of the palette: a name, grey detail on the right, and what it does.
#[derive(Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    pub detail: String,
    pub action: Action,
    pub glyph: Option<Glyph>,
    /// The registry word the query found it by ("aws" for Set frame
    /// type…), shown faintly after the name.
    pub via: &'static str,
    pub part: Part,
}

/// The parts of the ⌘K list, in order: commands, then kit types to add,
/// then the open file's nodes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Part {
    #[default]
    Main,
    Types,
    Nodes,
}

/// What [`List::Commands`] also searches, as entries: a type to add has
/// its kind's glyph ([`Part::Types`]), a node none ([`Part::Nodes`]).
pub fn extra_part(e: &Entry) -> Part {
    if matches!(e.glyph, Some(Glyph::Kind(_))) { Part::Types } else { Part::Nodes }
}

/// The rows of `list` matching `query`, best first. Commands are offered
/// only when `selected` fits them. Lists other than commands and templates
/// show `entries`; choosing one is [`Action::Pick`] with its index.
///
/// Commands rank by tier: the query in the name, then in one of the
/// command's [`Command::words`], then spread through the name ("zi" for
/// Zoom in). After them, for a typed query, the `entries` given with the
/// commands: kit types, then nodes, matched by their label.
pub fn rows(list: List, query: &str, selected: Selection, entries: &[Entry]) -> Vec<Row> {
    let row = |name: String, detail: String, action, glyph, part| Row { name, detail, action, glyph, via: "", part };
    let all: Vec<(Row, &'static str)> = match list {
        List::Commands | List::Shortcuts => COMMANDS
            .iter()
            .filter(|c| c.action != Action::Palette)
            .filter(|c| list == List::Shortcuts || c.needs.met_by(selected))
            .map(|c| (row(c.name.into(), c.keys.into(), c.action, None, Part::Main), c.words))
            .collect(),
        List::Templates => rhizo_lang::starters::STARTERS
            .iter()
            .map(|&(kind, name)| (row(name.into(), String::new(), Action::New(kind), None, Part::Main), ""))
            .collect(),
        _ => entries
            .iter()
            .enumerate()
            .map(|(i, e)| (row(e.name.clone(), e.detail.clone(), Action::Pick(i), e.glyph, Part::Main), ""))
            .collect(),
    };
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let qs: String = q.iter().collect();
    let squash = |t: &str| t.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect::<String>();
    let text = |r: &Row| {
        if list.searches_detail() { format!("{} {}", r.name, r.detail).to_lowercase() } else { r.name.to_lowercase() }
    };
    // (tier, score, index): lower is better.
    let mut hits: Vec<((u8, usize, usize), Row)> = Vec::new();
    for (i, (mut r, words)) in all.into_iter().enumerate() {
        let t = text(&r);
        let rank = if qs.is_empty() {
            Some((0, 0))
        } else if let Some(at) = squash(&t).find(&qs) {
            Some((0, at))
        } else if let Some((k, w)) = words.split(' ').enumerate().find(|(_, w)| w.contains(qs.as_str())) {
            // Whole words, not letters spread out: "aws" must not find every name with an a, w and s.
            r.via = w;
            Some((1, k))
        } else {
            score(&q, &t).map(|s| (2, s))
        };
        if let Some((tier, s)) = rank {
            hits.push(((tier, s, i), r));
        }
    }
    if list == List::Commands && !qs.is_empty() {
        for (i, e) in entries.iter().enumerate() {
            let Some(at) = e.name.to_lowercase().find(&query.trim().to_lowercase()) else { continue };
            let part = extra_part(e);
            let name = if part == Part::Types { format!("Add typed node: {}", e.name) } else { e.name.clone() };
            hits.push((
                (2 + part as u8, at, i),
                Row { part, ..row(name, e.detail.clone(), Action::Pick(i), e.glyph, part) },
            ));
        }
    }
    hits.sort_by_key(|(k, _)| *k);
    hits.into_iter().map(|(_, r)| r).collect()
}

/// Lower is better: position of the first match plus the gaps between matches.
fn score(q: &[char], name: &str) -> Option<usize> {
    let mut chars = name.chars().filter(|c| !c.is_whitespace()).enumerate();
    let mut first = None;
    let mut last = 0;
    let mut gaps = 0;
    for &want in q {
        let (i, _) = chars.find(|&(_, c)| c == want)?;
        if first.is_some() {
            gaps += i - last - 1;
        } else {
            first = Some(i);
        }
        last = i;
    }
    Some(first.unwrap_or(0) + gaps)
}

/// Rows the palette shows at once; more scroll.
const SHOWN: usize = 8;

/// Draws the palette over a `size` (pixels) window: the query, then `rows`
/// with `index` chosen. Returns where each shown row is, with its index.
#[allow(clippy::too_many_arguments)]
pub fn draw(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    (list, query, index): (List, &str, usize),
    rows: &[Row],
) -> Vec<(Rect, usize)> {
    let mut field = Field::new(query);
    field.buffer.set_caret(query.len(), false);
    draw_query(s, fonts, cache, size, dpr, bg, (list, &field, index), rows)
}

/// [`draw`], with the query as a field: its caret and selection.
#[allow(clippy::too_many_arguments)]
pub fn draw_query(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    size: Vec2,
    dpr: f64,
    bg: Color,
    (list, query, index): (List, &Field, usize),
    rows: &[Row],
) -> Vec<(Rect, usize)> {
    let fg = chrome::ink(bg);
    let (w, row) = ((560.0 * dpr).min(size.x - 32.0 * dpr), 34.0 * dpr);
    let shown = rows.len().min(SHOWN);
    let x0 = (size.x - w) / 2.0;
    let y0 = 80.0 * dpr;
    let panel = Rect::new(x0, y0, x0 + w, y0 + row * (shown as f64 + 1.0) + 12.0 * dpr);
    let rr = RoundedRect::from_rect(panel, 12.0 * dpr);
    let mut shadow = Color::rgb(0, 0, 0);
    shadow.a = 50;
    s.draw_blurred_rounded_rect(
        Affine::translate((0.0, 6.0 * dpr)),
        panel,
        peniko_color(shadow),
        12.0 * dpr,
        12.0 * dpr,
    );
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg), None, &rr);
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(fg.mix(bg, 0.8)), None, &rr);
    let size_pt = 15.0 * dpr as f32;
    let pad = 16.0 * dpr;
    let base = |i: f64| (y0 + 6.0 * dpr + row * (i + 0.5)) as f32 + size_pt * 0.35;
    if query.text().is_empty() && query.preedit.is_empty() {
        let placeholder = list.placeholder();
        draw::text(
            s,
            fonts,
            cache,
            Affine::IDENTITY,
            placeholder,
            (x0 + pad) as f32,
            base(0.0),
            size_pt,
            500,
            fg.mix(bg, 0.5),
        );
        let cx = x0 + pad;
        let caret = Line::new((cx, base(0.0) as f64 - size_pt as f64), (cx, base(0.0) as f64 + 3.0 * dpr));
        s.stroke(&Stroke::new(1.5 * dpr), Affine::IDENTITY, peniko_color(ACCENT), None, &caret);
    } else {
        let look = field::Look { size: size_pt, weight: 500, ink: fg, focused: true, px: dpr };
        field::draw(s, fonts, cache, query, x0 + pad, base(0.0) as f64, w - 2.0 * pad, &look);
    }
    let glyphs = rows.iter().any(|r| r.glyph.is_some());
    let indent = if glyphs { 24.0 * dpr } else { 0.0 };
    // Scrolled so the chosen row shows.
    let first = index.saturating_sub(shown.saturating_sub(1));
    let mut rects = Vec::with_capacity(shown);
    for (i, c) in rows.iter().enumerate().skip(first).take(shown) {
        let y = (i - first) as f64 + 1.0;
        let r =
            Rect::new(x0 + 6.0 * dpr, y0 + 6.0 * dpr + row * y, x0 + w - 6.0 * dpr, y0 + 6.0 * dpr + row * (y + 1.0));
        rects.push((r, i));
        if i == index {
            let mut hl = ACCENT;
            hl.a = 36;
            s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(hl), None, &RoundedRect::from_rect(r, 8.0 * dpr));
        }
        if let Some(g) = c.glyph {
            sidebar::glyph(s, g, Point::new(x0 + pad + 7.0 * dpr, r.center().y), dpr, bg);
        }
        let room = w - 2.0 * pad - indent;
        let keys = chrome::keys(fonts, &c.detail);
        let kw = fonts.shape(&keys, size_pt * 0.85, 400).map_or(0.0, |k| k.width) as f64;
        // The name gets at least 60% of the room; the detail what is left.
        let name = chrome::clip(fonts, &c.name, size_pt, 400, (room - kw - 24.0 * dpr).max(room * 0.6));
        let name_w =
            draw::text(s, fonts, cache, Affine::IDENTITY, &name, (x0 + pad + indent) as f32, base(y), size_pt, 400, fg)
                as f64;
        // The word that found it, faint: why "aws" lists Set frame type….
        let name_w = if c.via.is_empty() {
            name_w
        } else {
            let via =
                chrome::clip(fonts, &format!("  {}", c.via), size_pt * 0.85, 400, room - name_w - kw - 24.0 * dpr);
            let x = (x0 + pad + indent + name_w) as f32;
            name_w
                + draw::text(s, fonts, cache, Affine::IDENTITY, &via, x, base(y), size_pt * 0.85, 400, fg.mix(bg, 0.6))
                    as f64
        };
        // Kit types and nodes start their own part of the list.
        if i > first && rows[i - 1].part != c.part {
            let l = Line::new((r.x0 + 8.0 * dpr, r.y0), (r.x1 - 8.0 * dpr, r.y0));
            s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(fg.mix(bg, 0.85)), None, &l);
        }
        let keys = chrome::clip(fonts, &keys, size_pt * 0.85, 400, room - name_w - 24.0 * dpr);
        let kw = fonts.shape(&keys, size_pt * 0.85, 400).map_or(0.0, |k| k.width) as f64;
        let (kx, ky) = ((x0 + w - pad - kw) as f32, base(y));
        draw::text(s, fonts, cache, Affine::IDENTITY, &keys, kx, ky, size_pt * 0.85, 400, fg.mix(bg, 0.45));
    }
    rects
}

/// One line of a context menu: an empty name is a separator.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuRow {
    pub name: String,
    pub keys: String,
    pub enabled: bool,
}

/// Draws a context menu at `at` (window pixels), styled like the palette
/// and kept inside a `size` window, `hover` highlighted. Returns where each
/// enabled row is, with its index.
#[allow(clippy::too_many_arguments)]
pub fn draw_menu(
    s: &mut Scene,
    fonts: &Fonts,
    cache: &mut FontCache,
    at: Vec2,
    size: Vec2,
    dpr: f64,
    bg: Color,
    rows: &[MenuRow],
    hover: Option<usize>,
) -> Vec<(Rect, usize)> {
    let fg = chrome::ink(bg);
    let pt = 13.0 * dpr as f32;
    let (row, sep, pad) = (26.0 * dpr, 9.0 * dpr, 12.0 * dpr);
    let width = |t: &str, k: f32| fonts.shape(t, pt * k, 400).map_or(0.0, |s| s.width as f64);
    let keys: Vec<String> = rows.iter().map(|r| chrome::keys(fonts, &r.keys)).collect();
    let name_w = rows.iter().map(|r| width(&r.name, 1.0)).fold(0.0, f64::max);
    let keys_w = keys.iter().map(|k| width(k, 0.92)).fold(0.0, f64::max);
    let w = (pad + name_w + 32.0 * dpr + keys_w + pad).max(200.0 * dpr).min(size.x - 8.0 * dpr);
    let h = rows.iter().map(|r| if r.name.is_empty() { sep } else { row }).sum::<f64>() + 10.0 * dpr;
    // Right of and below the pointer, flipped where the window ends.
    let x0 = if at.x + w > size.x - 4.0 * dpr { (at.x - w).max(4.0 * dpr) } else { at.x };
    let y0 = if at.y + h > size.y - 4.0 * dpr { (at.y - h).max(4.0 * dpr) } else { at.y };
    let panel = Rect::new(x0, y0, x0 + w, y0 + h);
    let rr = RoundedRect::from_rect(panel, 8.0 * dpr);
    chrome::shadow(s, panel, 8.0 * dpr, dpr);
    s.fill(Fill::NonZero, Affine::IDENTITY, peniko_color(bg), None, &rr);
    s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(fg.mix(bg, 0.8)), None, &rr);
    let mut y = y0 + 5.0 * dpr;
    let mut rects = Vec::new();
    for (i, (r, k)) in rows.iter().zip(&keys).enumerate() {
        if r.name.is_empty() {
            let l = Line::new((x0 + pad / 2.0, y + sep / 2.0), (x0 + w - pad / 2.0, y + sep / 2.0));
            s.stroke(&Stroke::new(dpr), Affine::IDENTITY, peniko_color(fg.mix(bg, 0.85)), None, &l);
            y += sep;
            continue;
        }
        let line = Rect::new(x0 + 5.0 * dpr, y, x0 + w - 5.0 * dpr, y + row);
        if r.enabled {
            rects.push((line, i));
        }
        if hover == Some(i) && r.enabled {
            s.fill(
                Fill::NonZero,
                Affine::IDENTITY,
                peniko_color(ACCENT),
                None,
                &RoundedRect::from_rect(line, 5.0 * dpr),
            );
        }
        let ink = match () {
            _ if !r.enabled => fg.mix(bg, 0.6),
            _ if hover == Some(i) => Color::rgb(255, 255, 255),
            _ => fg,
        };
        let base = (line.center().y as f32) + pt * 0.35;
        draw::text(s, fonts, cache, Affine::IDENTITY, &r.name, (x0 + pad) as f32, base, pt, 400, ink);
        let kx = x0 + w - pad - width(k, 0.92);
        draw::text(s, fonts, cache, Affine::IDENTITY, k, kx as f32, base, pt * 0.92, 400, ink.mix(bg, 0.4));
        y += row;
    }
    rects
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(q: &str, sel: Selection) -> Vec<String> {
        rows(List::Commands, q, sel, &[]).into_iter().map(|r| r.name).collect()
    }

    #[test]
    fn filters_and_ranks() {
        use Selection::*;
        // The palette does not list itself.
        let count = |n: Needs| COMMANDS.iter().filter(|c| c.needs == n && c.action != Action::Palette).count();
        let all = COMMANDS.len() - 1;
        let (edge, frame) = (count(Needs::Edge), count(Needs::Frame));
        assert_eq!(names("", Node).len(), all - edge - count(Needs::Many) - frame, "edge and frame commands hidden");
        let no_nodes = all - count(Needs::Node) - count(Needs::AnyNodes) - count(Needs::Many) - count(Needs::Boxes);
        assert_eq!(names("", Nothing).len(), no_nodes - edge - frame);
        assert_eq!(names("", NoDocument).len(), count(Needs::Always), "only file commands without a document");
        let many = all - count(Needs::Node) - edge - frame;
        assert_eq!(names("", Nodes).len(), many, "two nodes: what works on all of them");
        let on_frame = all - count(Needs::Node) - count(Needs::AnyNodes) - count(Needs::Many) - edge;
        assert_eq!(names("", Frame).len(), on_frame, "a frame: frame commands and the look");
        assert_eq!(names("ungroup", Frame), ["Ungroup frame"]);
        assert!(names("ungroup", Node).is_empty());
        assert_eq!(names("group", Nodes), ["Group into frame", "New frame"], "then by a word: a frame groups");
        assert_eq!(names("frame type", Frame), ["Set frame type…"]);
        assert_eq!(names("del", Frame)[0], "Delete frame…");
        assert_eq!(names("about", NoDocument), ["About rhizo"]);
        assert_eq!(names("align left", Nodes), vec!["Align left edges"]);
        assert!(names("align", Node).is_empty(), "aligning needs two nodes");
        assert_eq!(names("del", Nodes)[0], "Delete");
        assert_eq!(names("zi", Nothing)[0], "Zoom in");
        assert_eq!(names("del", Node)[0], "Delete");
        assert_eq!(names("del", Edge)[0], "Delete edge");
        assert_eq!(names("label", Edge), vec!["Edit edge label", "Find node…"]);
        assert_eq!(names("ADD", Edge), vec!["Add typed node…"], "no child or sibling of an edge");
        assert_eq!(
            names("ADD", Node),
            vec!["Add child", "Add sibling", "Add typed node…", "Add acceptance criterion", "Adopt node…"]
        );
        assert_eq!(names("type", Node), vec!["Add typed node…", "Change type…", "Kits in use…", "Toggle inspector"]);
        assert_eq!(
            names("type", Nothing),
            vec!["Add typed node…", "Kits in use…", "Toggle inspector"],
            "changing needs a node"
        );
        assert_eq!(names("inspector", Nothing), vec!["Toggle inspector"]);
        assert_eq!(names("fit", Nothing)[0], "Fit to window");
        assert_eq!(names("export", Nothing)[0], "Export…");
        assert!(names("export", NoDocument).is_empty());
        assert_eq!(names("open", NoDocument), vec!["Open…", "Open folder…", "Open recent…", "Open sample…"]);
        assert!(names("xyz", Node).is_empty());
    }

    /// What a person calls a thing finds the command: names first, then
    /// the registry's words, then letters spread through a name.
    #[test]
    fn synonyms_find_commands_after_names() {
        use Selection::*;
        let found = |q: &str, sel| -> Vec<(String, &'static str)> {
            rows(List::Commands, q, sel, &[]).into_iter().map(|r| (r.name, r.via)).collect()
        };
        for q in ["environment", "cloud", "aws", "azure", "gcp"] {
            let f = found(q, Frame);
            assert!(f.iter().take(2).any(|(n, _)| n == "Set frame type…"), "{q}: {f:?}");
            assert!(f.iter().take(2).any(|(n, _)| n == "New frame"), "{q}: {f:?}");
            assert!(f.iter().any(|(n, via)| n == "Set frame type…" && via.contains(q)), "{q}: the word shows");
        }
        for q in ["group", "box", "container"] {
            assert_eq!(found(q, Nodes)[0].0, "Group into frame", "{q}");
        }
        for q in ["connect", "link", "arrow", "edge"] {
            assert_eq!(found(q, Node)[0].0, "Connect to…", "{q}");
            let on_edge: Vec<String> = found(q, Edge).into_iter().map(|r| r.0).collect();
            assert!(on_edge.iter().any(|n| n.contains("edge") || n.contains("arrow")), "{q}: {on_edge:?}");
        }
        for q in ["export", "png", "svg"] {
            assert_eq!(found(q, Nothing)[0].0, "Export…", "{q}");
        }
        assert_eq!(found("png", Nothing)[0].1, "png");
        assert_eq!(found("export", Nothing)[0].1, "", "a name match shows no word");
        for q in ["deploy", "terraform", "tofu", "openapi", "spec"] {
            assert_eq!(found(q, Nothing)[0].0, "Compile specs…", "{q}");
        }
        assert_eq!(found("kit", Node)[0].0, "Kits in use…");
        assert!(found("kit", Node).iter().any(|r| r.0 == "Add typed node…"));
        assert_eq!(found("undo", Node)[0].0, "Undo");
        assert_eq!(found("tutorial", NoDocument)[0].0, "Learn rhizo");
        assert_eq!(found("gallery", NoDocument)[0].0, "Open sample…");
        // A name match ranks before a word match: "frame" is in names.
        let f = found("frame", Frame);
        let first_word = f.iter().position(|r| !r.1.is_empty()).unwrap_or(f.len());
        assert!(f[..first_word].iter().all(|r| r.0.to_lowercase().contains("frame")), "{f:?}");
    }

    /// With the commands, ⌘K also finds kit types to add and the file's
    /// nodes by label, each in its own part after the commands.
    #[test]
    fn kit_types_and_nodes_by_label() {
        let kind = Glyph::Kind(crate::project::Kind::of("infra.bucket"));
        let extras = [
            Entry { name: "Bucket".into(), detail: "infra.bucket · infra kit".into(), glyph: Some(kind) },
            Entry { name: "Function".into(), detail: "infra.function · infra kit".into(), glyph: Some(kind) },
            Entry { name: "Upload bucket".into(), detail: "node · uploads".into(), glyph: None },
            Entry { name: "Checkout".into(), detail: "node · checkout".into(), glyph: None },
        ];
        let r = rows(List::Commands, "bucket", Selection::Nothing, &extras);
        let names: Vec<&str> = r.iter().map(|r| r.name.as_str()).collect();
        let typed = names.iter().position(|n| *n == "Add typed node: Bucket").expect("the type");
        let node = names.iter().position(|n| *n == "Upload bucket").expect("the node");
        assert!(typed < node, "types before nodes: {names:?}");
        assert_eq!(r[typed].action, Action::Pick(0));
        assert_eq!((r[typed].part, r[node].part), (Part::Types, Part::Nodes));
        assert!(r[..typed].iter().all(|r| r.part == Part::Main), "commands first: {names:?}");
        let r = rows(List::Commands, "check", Selection::Nothing, &extras);
        assert!(r.iter().any(|r| r.name == "Checkout" && r.action == Action::Pick(3)), "a label jumps to its node");
        assert!(
            rows(List::Commands, "", Selection::Nothing, &extras).iter().all(|r| r.part == Part::Main),
            "typed only"
        );
        assert!(rows(List::Commands, "zzz", Selection::Nothing, &extras).is_empty());
    }

    /// The status bar's hints follow the selection, from the registry's keys.
    #[test]
    fn hints_per_context() {
        use Selection::*;
        assert_eq!(hints(Node, false), ["Tab child", "Enter sibling", "F2 rename", "⌘I inspect"]);
        assert_eq!(hints(Nodes, false), ["⌘G group", "⌫ delete", "⌘C copy"]);
        assert_eq!(hints(Frame, false)[..2], ["Set frame type…", "Add typed node…"]);
        assert_eq!(hints(Edge, false)[..2], ["F2 label", "⌫ delete"]);
        assert_eq!(hints(Nothing, false), ["Tab add a node", "⌘F find", "⌘K commands"]);
        assert!(hints(NoDocument, false).is_empty());
        assert_eq!(hints(Node, true), ["⌘S save", "⌘Z undo", "⌘\\ hide text"], "the pane has its own");
        for sel in [Nothing, Node, Nodes, Edge, Frame] {
            let h = hints(sel, false);
            assert!(!h.is_empty() && h.len() <= 4, "{sel:?}: {h:?}");
            // Each hint's command is offered for this selection (⌘K and Tab on empty canvas aside).
            for text in &h {
                let c = COMMANDS.iter().find(|c| text.starts_with(c.keys) && !c.keys.is_empty() || *text == c.name);
                assert!(c.is_some(), "{text}: a command of the registry");
            }
        }
    }

    #[test]
    fn the_registry_names_each_chord_once() {
        let chords: Vec<_> = COMMANDS.iter().filter_map(|c| chord(c.keys)).collect();
        for (i, c) in chords.iter().enumerate() {
            assert!(!chords[..i].contains(c), "{c:?} twice");
        }
        assert_eq!(chord("⇧⌘Z"), Some((true, false, "z".into())));
        assert_eq!(chord("⌥⌘O"), Some((false, true, "o".into())));
        assert_eq!(chord("⌘\\"), Some((false, false, "\\".into())));
        assert_eq!(chord("⌥ drag"), None);
        assert_eq!(chord("Tab"), None);
        assert_eq!(command(Action::Undo).map(|c| c.keys), Some("⌘Z"));
        assert_eq!(tip(Action::ToggleInspector), "Toggle inspector   ⌘I");
        assert_eq!(tip(Action::Compile), "Compile specs…");
        // The shortcuts list shows every command, the palette only what fits.
        let all = rows(List::Shortcuts, "", Selection::NoDocument, &[]);
        assert_eq!(all.len(), COMMANDS.len() - 1, "all but the palette itself");
        assert!(all.iter().any(|r| r.name == "Delete edge" && r.detail == "⌫"));
    }

    #[test]
    fn templates_and_recent_files() {
        let t = rows(List::Templates, "flow", Selection::NoDocument, &[]);
        assert_eq!((t[0].name.as_str(), t[0].action), ("Flowchart", Action::New("flow")));
        assert_eq!(rows(List::Templates, "", Selection::Node, &[]).len(), 4);
        let recent = path_entries(&[PathBuf::from("/a/plan.rhizo"), PathBuf::from("/b/notes.rhizo")]);
        let r = rows(List::Recent, "notes", Selection::NoDocument, &recent);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].name.as_str(), r[0].action), ("notes.rhizo", Action::Pick(1)));
        assert_eq!(r[0].glyph, Some(Glyph::File));
        assert_eq!(rows(List::Recent, "", Selection::Nothing, &recent)[0].action, Action::Pick(0));
    }

    #[test]
    fn project_files_and_navigation() {
        let files = path_entries(&[PathBuf::from("/shop/api.rhizo"), PathBuf::from("/shop/flows/checkout.rhizo")]);
        let r = rows(List::Project, "check", Selection::Nothing, &files);
        assert_eq!((r[0].name.as_str(), r[0].action), ("checkout.rhizo", Action::Pick(1)));
        assert!(rows(List::Project, "shop", Selection::Nothing, &files).is_empty(), "file names only");
        assert_eq!(names("usages", Selection::Node), vec!["Find usages"]);
        assert!(names("usages", Selection::Nothing).is_empty(), "needs a node");
        assert_eq!(names("problems", Selection::Nothing), vec!["Show problems in the project"]);
        assert_eq!(names("folder", Selection::NoDocument), vec!["Open folder…"]);
        assert_eq!(names("definition", Selection::Node), vec!["Go to definition"]);
        assert!(names("definition", Selection::Nothing).is_empty(), "needs a node");
        assert_eq!(names("back", Selection::Nothing), vec!["Back"]);
        assert_eq!(names("text pane", Selection::Nothing), vec!["Toggle text pane"]);
        assert_eq!(names("revert", Selection::Node)[0], "Revert text to the file on disk");
        assert_eq!(
            names("adopt", Selection::Node),
            ["Adopt node…", "Adopt all that switch without changes", "Cancel adopt"]
        );
        assert_eq!(names("adopt", Selection::Nothing), ["Adopt all that switch without changes", "Cancel adopt"]);
        assert!(names("adopt", Selection::NoDocument).is_empty());
    }

    #[test]
    fn entry_lists_match_their_detail_too() {
        let e =
            |name: &str, detail: &str| Entry { name: name.into(), detail: detail.into(), glyph: Some(Glyph::Error) };
        let problems = [e("no node `ordr`", "flows/checkout.rhizo:12"), e("unknown prop `shap`", "api.rhizo:4")];
        let r = rows(List::Problems, "api", Selection::Nothing, &problems);
        assert_eq!(r.len(), 1);
        assert_eq!((r[0].action, r[0].detail.as_str()), (Action::Pick(1), "api.rhizo:4"));
        assert_eq!(rows(List::Problems, "", Selection::Nothing, &problems).len(), 2, "all, in order");
        assert_eq!(rows(List::References, "ordr", Selection::Nothing, &problems)[0].action, Action::Pick(0));
    }
}
