//! Who makes rhizo and where it lives: the facts the viewer's welcome
//! footer and About dialog, the README and `rhizo about` share.

/// The public home of the code. One constant, so a move is one edit.
pub const GITHUB: &str = "https://github.com/vxrz/rhizo";

pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The licence, as `Cargo.toml` states it.
pub const LICENSE: &str = "MIT or Apache-2.0";

/// What rhizo is, in one line.
pub const TAGLINE: &str = "Diagrams as text that you and your agents can build from.";

/// The first footer row up to the Franconian rake, and what follows it.
pub const MADE_IN: &str = "Made with ❤️ in Erlangen, Germany · Metropolregion Nürnberg";
pub const BUILT_BY: &str = "Designed & built by uebelacker.io with AI";

/// The second footer row: copyright and licence before the links.
pub const COPYRIGHT: &str = "© 2026 uebelacker.io";

/// The footer as two plain lines (a terminal cannot draw the rake), with
/// `github` standing where the "GitHub" link goes.
pub fn footer_lines(github: &str) -> [String; 2] {
    [
        format!("{MADE_IN} · {BUILT_BY}"),
        format!("{COPYRIGHT} · Open source ({LICENSE}) · {github} · rhizo {VERSION} · About rhizo"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_footer_names_the_licence_and_version() {
        let [one, two] = footer_lines("GitHub");
        assert!(one.contains("Erlangen, Germany · Metropolregion Nürnberg · Designed & built"));
        assert!(two.contains("Open source (MIT or Apache-2.0) · GitHub · rhizo "));
        assert!(two.ends_with(&format!("rhizo {VERSION} · About rhizo")));
    }
}
