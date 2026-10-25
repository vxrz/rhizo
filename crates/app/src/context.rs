//! What a context menu lists for the place right-clicked: commands from
//! the registry, and for sidebar rows, what can be done with the file or
//! thing. The window draws them with [`crate::palette::draw_menu`].

use std::path::PathBuf;

use crate::palette::{self, Action, Arrange as How, MenuRow, Selection};

/// One line of a context menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    Run(Action),
    /// Opens a file of the project.
    Open(PathBuf),
    /// Shows a file in the file manager.
    Reveal(PathBuf),
    /// Goes to a thing of the sidebar, by its index.
    GoTo(usize),
    /// Puts a reference to a thing of the sidebar into the open file.
    Refer(usize),
    Separator,
}

/// What a right-click was on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// One node (a reference, or not), or one of several selected.
    Node {
        reference: bool,
        many: bool,
        /// Imported code that is not adopted yet: "Adopt node…" applies.
        code: bool,
    },
    Edge,
    /// An explicit frame (its label strip or its empty space).
    Frame,
    Canvas,
    Pane,
    File(PathBuf),
    Thing(usize),
}

/// The menu for `place`, from the registry.
pub fn items(place: &Place) -> Vec<Item> {
    use Action::*;
    let run = |a: &[Action]| a.iter().map(|&a| Item::Run(a)).collect::<Vec<_>>();
    let sep = || vec![Item::Separator];
    match place {
        Place::Node { many: true, .. } => [
            run(&[Cut, Copy, Paste, Delete]),
            sep(),
            run(&[GroupFrame, ChangeTypeMenu, ShapeMenu, FillMenu, StrokeMenu]),
            sep(),
            run(&[Arrange(How::Left), Arrange(How::Center), Arrange(How::Right)]),
            run(&[Arrange(How::Top), Arrange(How::Middle), Arrange(How::Bottom)]),
            run(&[Arrange(How::SpreadX), Arrange(How::SpreadY)]),
        ]
        .concat(),
        Place::Node { reference, code, .. } => [
            run(&[Rename, AddChild, AddSibling]),
            sep(),
            run(&[Cut, Copy, Paste, Delete]),
            sep(),
            run(&[ChangeTypeMenu, Connect, FindUsages, GroupFrame]),
            sep(),
            run(&[ShapeMenu, FillMenu, StrokeMenu, AddAccept]),
            if *reference {
                run(&[GoToDefinition])
            } else if *code {
                run(&[AdoptNode])
            } else {
                Vec::new()
            },
        ]
        .concat(),
        Place::Edge => [run(&[Rename, Delete]), sep(), run(&[EdgeKindMenu, LineMenu, ArrowMenu])].concat(),
        Place::Frame => [
            run(&[Rename, FrameTypeMenu, LayoutMenu, DirectionMenu]),
            sep(),
            run(&[AddTypedMenu, NewFrame, FillMenu, StrokeMenu]),
            sep(),
            run(&[Ungroup, Delete]),
        ]
        .concat(),
        Place::Canvas => [
            run(&[Paste, SelectAll]),
            sep(),
            run(&[NewFrame, AddTypedMenu, InsertReferenceMenu]),
            sep(),
            run(&[Fit, ZoomIn, ZoomOut, TidyLayout]),
        ]
        .concat(),
        Place::Pane => [run(&[Cut, Copy, Paste, SelectAll]), sep(), run(&[TogglePane])].concat(),
        Place::File(p) => vec![Item::Open(p.clone()), Item::Reveal(p.clone())],
        Place::Thing(i) => vec![Item::GoTo(*i), Item::Refer(*i)],
    }
}

/// How a command reads in a menu, with its keys, and whether `selection`
/// lets it run. Delete and Rename have an edge form, picked for an edge.
pub fn row(action: Action, selection: Selection) -> MenuRow {
    let c = palette::COMMANDS.iter().find(|c| c.action == action && c.needs.met_by(selection));
    let enabled = c.is_some();
    let c = c.or_else(|| palette::command(action)).expect("every menu action is a command");
    MenuRow { name: c.name.into(), keys: c.keys.into(), enabled }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(place: Place) -> Vec<&'static str> {
        items(&place)
            .iter()
            .filter_map(|i| match i {
                Item::Run(a) => palette::command(*a).map(|c| c.name),
                Item::Separator => Some("—"),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn each_place_lists_what_can_be_done_there() {
        assert_eq!(names(Place::Edge)[..2], ["Rename", "Delete"], "shown as Edit edge label and Delete edge");
        assert!(names(Place::Edge).contains(&"Set edge kind…"));
        let frame = names(Place::Frame);
        assert!(["Set frame type…", "Set layout…", "Ungroup frame", "New frame"].iter().all(|n| frame.contains(n)));
        assert_eq!(row(Action::Delete, Selection::Frame).name, "Delete frame…");
        assert_eq!(row(Action::Rename, Selection::Frame).name, "Rename frame");
        assert!(names(Place::Canvas).contains(&"New frame"));
        let node = names(Place::Node { reference: false, many: false, code: true });
        assert_eq!(&node[..3], ["Rename", "Add child", "Add sibling"]);
        assert!(node.contains(&"Adopt node…") && !node.contains(&"Go to definition"));
        let reference = names(Place::Node { reference: true, many: false, code: false });
        let plain = names(Place::Node { reference: false, many: false, code: false });
        assert!(!plain.contains(&"Adopt node…"), "only imported code can be adopted");
        assert!(reference.contains(&"Go to definition") && !reference.contains(&"Adopt node…"));
        let many = names(Place::Node { reference: false, many: true, code: false });
        assert!(many.contains(&"Align left edges") && many.contains(&"Distribute vertically"));
        assert!(!many.contains(&"Rename"), "renaming is for one node");
        assert_eq!(names(Place::Canvas)[..2], ["Paste", "Select all"]);
        assert_eq!(names(Place::Pane), ["Cut", "Copy", "Paste", "Select all", "—", "Toggle text pane"]);
        assert_eq!(items(&Place::Thing(3)), [Item::GoTo(3), Item::Refer(3)]);
        assert_eq!(row(Action::Delete, Selection::Edge).name, "Delete edge");
        assert_eq!(row(Action::Rename, Selection::Edge).name, "Edit edge label");
        let paste = row(Action::Paste, Selection::Nothing);
        assert_eq!((paste.keys.as_str(), paste.enabled), ("⌘V", true));
        assert!(!row(Action::Arrange(How::Left), Selection::Node).enabled, "aligning needs two nodes");
        // Every action a menu lists is in the registry, with the keys it shows.
        for p in [
            Place::Node { reference: true, many: true, code: false },
            Place::Node { reference: false, many: false, code: true },
        ] {
            for i in items(&p) {
                if let Item::Run(a) = i {
                    assert!(palette::command(a).is_some(), "{a:?} is a command");
                }
            }
        }
    }
}
