//! `rhizo-view [FILE|FOLDER] [--theme light|dark|paper]`: a live window onto
//! a rhizo document. Edit the file anywhere and the window follows; edit in
//! the window, on the canvas or in its text pane, and the file follows (KDL
//! files only). A sidebar shows the project around it: its files, their
//! typed things and problems. Without a file it opens on a welcome screen.

mod viewer;

use std::path::PathBuf;

use winit::event_loop::EventLoop;

use viewer::{App, Event};

fn main() {
    let mut args = std::env::args().skip(1);
    let mut path = None;
    let mut theme = None;
    while let Some(a) = args.next() {
        match a.as_str() {
            "--theme" => theme = args.next(),
            "-h" | "--help" => {
                help();
                return;
            }
            _ if path.is_none() => path = Some(PathBuf::from(a)),
            other => {
                eprintln!("unexpected argument `{other}` (see --help)");
                std::process::exit(2);
            }
        }
    }
    if let Some(t) = &theme
        && !rhizo_app::theme_names().contains(&t.as_str())
    {
        eprintln!("unknown theme `{t}` ({})", rhizo_app::theme_names().join(", "));
        std::process::exit(2);
    }
    let event_loop = EventLoop::<Event>::with_user_event().build().expect("event loop");
    let mut app = App::new(&event_loop, path, theme);
    event_loop.run_app(&mut app).expect("event loop");
}

fn help() {
    println!("usage: rhizo-view [FILE|FOLDER] [--theme {}]", rhizo_app::theme_names().join("|"));
    println!("learn: the welcome screen's Learn rhizo (or Help > Learn rhizo, or `rhizo new learn`) saves a");
    println!("       tutorial diagram and ticks its ten steps off as you do them · Samples (or File > Open");
    println!("       Sample…) opens a copy of a built-in example · the status bar suggests keys for the selection");
    println!("       (View > Status-Bar Hints hides them) · cmd+K also finds commands by other words (\"aws\",");
    println!("       \"png\", \"terraform\"), kit types to add (\"bucket\") and nodes by label · an empty document");
    println!("       says how to start: double-click or Tab adds a node");
    println!("files: without FILE, a welcome screen offers templates, the tutorial, samples, Open… and recent files");
    println!("       a FILE that does not exist yet can be created from a template; until then the");
    println!("       window waits and shows it as soon as it appears");
    println!("       cmd+N new from template · cmd+O open (Mermaid, Markdown, OPML, FreeMind and");
    println!("       JSON Canvas are imported into a new .rhizo file) · shift+cmd+O open recent");
    println!("       shift+cmd+S save as · cmd+E export (SVG, PNG, PDF, Mermaid, D2, DOT, draw.io,");
    println!("       JSON Canvas, OPML, FreeMind, GraphML, Markdown, JSON, YAML) · cmd+W close");
    println!("       drop a file on the window to open it · every edit is written to the file at once");
    println!("       cmd+K also reveals the file in the file manager or opens it in a text editor");
    println!("menus: on macOS the menu bar holds every command with its keys; cmd+K lists them anywhere,");
    println!("       cmd+/ shows every shortcut · right-click (ctrl+click on macOS) a node, an edge, the canvas,");
    println!("       a sidebar row or the text pane for what can be done there · rest the pointer on a control");
    println!("       for a tooltip with its keys · Diagram ▸ Compile specs…, Build brief · Help ▸ Guide");
    println!("focus: one part has the keys and an accent ring: canvas, text pane, inspector field, sidebar,");
    println!("       palette, menu or dialog · Esc steps back to the canvas · in the sidebar arrows move,");
    println!("       Enter opens, left and right fold");
    println!("view:  scroll pans · cmd/ctrl+scroll or pinch zooms · F or double-click empty space fits · 0 = 100%");
    println!("       space+drag or the middle button pans · View ▸ themes and Tidy layout");
    println!("       + and - zoom · edits and file changes animate into place");
    println!("       hover highlights what a click selects · a selected node lights up its edges");
    println!("       the status bar shows counts, zoom and the selection (hover the file name for its");
    println!("       path); problems in the file show in a banner while the last good diagram stays");
    println!("edit:  click selects · shift+click adds or removes · cmd+A selects all · a drag on empty canvas");
    println!("       draws a selection box · arrows move the selection · Tab adds a child · Enter a sibling");
    println!("       several nodes: drag moves them (free layouts), Backspace deletes, cmd+X/C/V cut, copy and");
    println!("       paste them as rhizo text (edges between them too), each as one undo step; Diagram ▸ Align");
    println!("       and Distribute line them up; the inspector sets a prop on all (\"mixed\" where they differ)");
    println!("       cmd+F finds a node");
    println!("       F2 or double-click renames · Backspace deletes · drag moves or re-parents");
    println!("       alt+drag (any drag in flowcharts) connects · cmd+K commands · cmd+Z / shift+cmd+Z");
    println!("fields: inspector fields, labels and the palette edit like the text pane: alt+arrows by word,");
    println!("       cmd+arrows to the ends, shift selects, double click a word, triple click all, cmd+A/C/X/V,");
    println!("       cmd+Z undoes typing before Enter writes it · Tab and shift+Tab go between inspector fields");
    println!("types: cmd+I shows the inspector: the selected node's type and props as a form from its kit's");
    println!("       schema (a list for enums, a switch for true/false, text fields checked as you type;");
    println!("       * marks required props); Set type… or cmd+K Change type… types a node (the kit's");
    println!("       `use` line is added), cmd+K Add typed node… adds one; Enter/Tab next to a typed");
    println!("       node in a typed frame or node (operations in an API) give the new node its type");
    println!("       the inspector also shows where a reference is defined, or which files use a node");
    println!("frames: right-click the canvas > New frame (inside a selected frame it nests) and type its name;");
    println!("       cmd+G groups the selected nodes into a frame · click a frame's label to select it, F2 renames,");
    println!("       Backspace asks: keep what it holds (Ungroup) or delete it all · drag a node onto another");
    println!("       frame to move it there, outside every frame to take it out · cmd+K Set frame type… (Environment,");
    println!("       API, Workflow…), Set layout…, Set direction…; the inspector shows the frame's props from its");
    println!("       type's schema and Add typed node… lists what fits in it");
    println!("look:  the inspector sets a node's shape, fill and stroke (swatches), note, link, tags, classes and");
    println!("       acceptance criteria; an edge's label, kind, line, arrowheads, colour, predicate and the props");
    println!("       its kits give it (access, prefix, route); with nothing selected, the document's title, theme,");
    println!("       kits and loose-node layout · Format menu and cmd+K have the same · deleting the last reference");
    println!("       into an import offers to remove it");
    println!("about: rhizo > About rhizo (or cmd+K) shows the version, licence and where rhizo is made");
    println!("adopt: on a node of imported infrastructure code the inspector's Adopt… (or cmd+K Adopt node…,");
    println!("       Adopt all that switch without changes) runs a dry run in the background (OpenTofu,");
    println!("       progress in the status bar, cmd+K Cancel adopt stops it), then shows its report;");
    println!("       Adopt (or cmd+Enter) writes it as one edit cmd+Z undoes, Esc drops it");
    println!("edges: click selects · Backspace removes · F2 or double-click edits the label (empty removes it)");
    println!("projects: a file can `import \"model.rhizo\" as=model` and use `model:order`; references");
    println!("       draw dashed with their target's label; edits to imported files redraw this one");
    println!("       cmd+click a reference (or cmd+K Go to definition) opens its file · cmd+[ back");
    println!("       cmd+P goes to another file in the folder · kit and reference problems show in the banner");
    println!("       a FOLDER (or alt+cmd+O) opens as a project; a file finds its project around it");
    println!("       cmd+1 toggles the sidebar: files, their typed things (click to go, drag onto the");
    println!("       canvas to refer to one; alt+drag a node onto one to connect) and problems");
    println!("       shift+cmd+M lists every problem in the project · cmd+R inserts a reference to a");
    println!("       typed node of another file (the import is added) · shift+cmd+U finds usages");
    println!("text:  cmd+\\ (or cmd+K Toggle text pane) shows the file's KDL beside the canvas, highlighted");
    println!("       and editable; drag the divider to resize; Esc hands the keys back to the canvas");
    println!("       typing redraws the canvas after a 200 ms pause and saves once the text parses;");
    println!("       problems are underlined and the last good diagram stays · cmd+S saves as typed");
    println!("       canvas edits show in the text at once; selecting a node puts the caret on its");
    println!("       statement, and the caret selects the node or edge it is in");
    println!("       one undo history (cmd+Z) for typing and canvas edits");
    println!("       if the file changes on disk while you type, your text stays: cmd+S keeps it,");
    println!("       cmd+K Revert text takes the file's");
    println!("       alt+arrows move by word, cmd+arrows to line or file ends, shift selects");
    println!("       cmd+A/C/X/V · Tab and shift+Tab indent · Enter keeps the indent");
    println!("(ctrl instead of cmd on Linux and Windows; ctrl+arrows move by word there)");
}
