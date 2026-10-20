//! The ⌘K command palette: every viewer action, filterable by typing.

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
    ZoomIn,
    ZoomOut,
}

pub struct Command {
    pub name: &'static str,
    pub keys: &'static str,
    pub action: Action,
    /// Needs a selected node.
    pub needs_selection: bool,
}

pub const COMMANDS: &[Command] = &[
    Command { name: "Add child", keys: "Tab", action: Action::AddChild, needs_selection: true },
    Command { name: "Add sibling", keys: "Enter", action: Action::AddSibling, needs_selection: true },
    Command { name: "Rename", keys: "F2", action: Action::Rename, needs_selection: true },
    Command { name: "Delete", keys: "⌫", action: Action::Delete, needs_selection: true },
    Command { name: "Connect to…", keys: "⌥ drag", action: Action::Connect, needs_selection: true },
    Command { name: "Undo", keys: "⌘Z", action: Action::Undo, needs_selection: false },
    Command { name: "Redo", keys: "⇧⌘Z", action: Action::Redo, needs_selection: false },
    Command { name: "Fit to window", keys: "F", action: Action::Fit, needs_selection: false },
    Command { name: "Zoom to 100%", keys: "0", action: Action::ZoomActual, needs_selection: false },
    Command { name: "Zoom in", keys: "+", action: Action::ZoomIn, needs_selection: false },
    Command { name: "Zoom out", keys: "−", action: Action::ZoomOut, needs_selection: false },
];

/// Commands matching `query` as a case-insensitive subsequence, best first:
/// earlier and tighter matches rank higher; ties keep the list order.
pub fn filter(query: &str, has_selection: bool) -> Vec<&'static Command> {
    let q: Vec<char> = query.to_lowercase().chars().filter(|c| !c.is_whitespace()).collect();
    let mut hits: Vec<(usize, usize, &'static Command)> = COMMANDS
        .iter()
        .enumerate()
        .filter(|(_, c)| has_selection || !c.needs_selection)
        .filter_map(|(i, c)| score(&q, &c.name.to_lowercase()).map(|s| (s, i, c)))
        .collect();
    hits.sort_by_key(|&(s, i, _)| (s, i));
    hits.into_iter().map(|(_, _, c)| c).collect()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn names(q: &str, sel: bool) -> Vec<&'static str> {
        filter(q, sel).iter().map(|c| c.name).collect()
    }

    #[test]
    fn filters_and_ranks() {
        assert_eq!(names("", true).len(), COMMANDS.len());
        assert_eq!(names("", false).len(), COMMANDS.len() - 5, "selection commands hidden");
        assert_eq!(names("zi", false)[0], "Zoom in");
        assert_eq!(names("del", true), vec!["Delete"]);
        assert_eq!(names("ADD", true), vec!["Add child", "Add sibling"]);
        assert_eq!(names("fit", false)[0], "Fit to window");
        assert!(names("xyz", true).is_empty());
    }
}
