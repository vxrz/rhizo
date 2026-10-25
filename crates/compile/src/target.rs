//! Compile targets: what each kit compiles to, and what that means for the
//! build brief.
//!
//! **Adding a kit with a compiler** takes:
//!
//! 1. its `kit.kdl` and `schemas/*.json` under `crates/kit/kits/<name>/`,
//!    embedded in `Kits::builtin`, and type-name constants in a
//!    `rhizo_kit::<name>` module;
//! 2. a `rhizo_kit::Rules` impl for what the schemas cannot say, and one
//!    line in `rhizo_kit::RULES`;
//! 3. a [`Target`] impl here, and one line in [`TARGETS`].
//!
//! The brief needs no change: it asks the targets what they generate, which
//! nodes still need work, how to name and check that work, and what is left
//! for a person to do.

use rhizo_core::{Document, Frame, Node};
use rhizo_kit::Finding;

use crate::Files;
use crate::brief::{Criterion, Task, TaskOut};
use crate::{data, flow, infra};

/// What a target compiles from.
pub(crate) struct Cx<'a> {
    pub doc: &'a Document,
    /// The source file name (not a path), for headers and ids.
    pub source_name: &'a str,
    /// The `@generated …` line for file headers.
    pub header: &'a str,
}

/// How the brief treats a target's files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Generated {
    /// Specs code must match: an input to the tasks about their nodes and
    /// those nodes' neighbors.
    Spec,
    /// Compiled workflows: specs too, but an input only to the tasks about
    /// their own nodes.
    Flow,
    /// Infrastructure a person applies.
    Infra,
}

/// A workflow a person deploys once it is compiled (and its open steps are
/// written).
pub(crate) struct Deploy<'a> {
    pub frame: &'a Frame,
    /// The node the task is about: its trigger, else its first step.
    pub node: Option<&'a Node>,
    /// Its steps, in document order.
    pub nodes: Vec<&'a Node>,
}

/// A compiler for one kit. Every hook but [`Target::emit`] serves the brief
/// and does nothing by default.
pub(crate) trait Target: Sync {
    /// The kit whose documents this compiles; it runs when a document uses it.
    fn kit(&self) -> &'static str;
    /// The bundle folder the target writes into (`specs`).
    fn dir(&self) -> &'static str;
    fn generated(&self) -> Generated;
    /// Writes the target's files. Errors stop the compile.
    fn emit(&self, cx: &Cx, files: &mut Files) -> Result<(), Vec<Finding>>;

    /// Whether `n` needs no task of its own: the target generates it
    /// completely, or it is built as part of another node's task.
    fn covers(&self, _doc: &Document, _n: &Node) -> bool {
        false
    }
    /// Nodes built as part of a task about `n` (the operations a function
    /// implements).
    fn parts<'a>(&self, _doc: &'a Document, _n: &Node) -> Vec<&'a Node> {
        Vec::new()
    }
    /// For a node whose task is writing code: that task's title.
    fn implement(&self, _n: &Node) -> Option<String> {
        None
    }
    /// Frames a person deploys after compiling (workflows).
    fn deploys<'a>(&self, _doc: &'a Document) -> Vec<Deploy<'a>> {
        Vec::new()
    }
    /// Acceptance criteria the target derives for a task; `inputs` are the
    /// task's generated input files.
    fn criteria(&self, _doc: &Document, _files: &Files, _t: &Task, _inputs: &[String]) -> Vec<Criterion> {
        Vec::new()
    }
    /// Tasks for a person after all others (apply the infrastructure);
    /// `order` is every other task, in order.
    fn human_tasks<'a>(&self, _doc: &'a Document, _files: &Files, _order: &[&Task<'a>]) -> Vec<TaskOut<'a>> {
        Vec::new()
    }
    /// What the diagram leaves open about this target's part of it.
    /// `decided` says whether a decision mentions any of the words.
    fn questions(&self, _doc: &Document, _decided: &dyn Fn(&[&str]) -> bool) -> Vec<String> {
        Vec::new()
    }
}

/// The built-in targets, in the order they compile (and report).
pub(crate) static TARGETS: &[&dyn Target] = &[&data::Data, &flow::Flow, &infra::Infra];

/// The target that writes `path`.
pub(crate) fn of_path(path: &str) -> Option<&'static dyn Target> {
    TARGETS.iter().copied().find(|t| path.strip_prefix(t.dir()).is_some_and(|rest| rest.starts_with('/')))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_target_compiles_a_builtin_kit() {
        let kits = rhizo_kit::Kits::builtin();
        for t in TARGETS {
            let kit = kits.get(t.kit()).unwrap_or_else(|| panic!("target for unknown kit `{}`", t.kit()));
            assert!(!kit.targets.is_empty(), "kit `{}` declares no targets but has a compiler", kit.name);
        }
        // Every kit that declares targets has a compiler, and each has one.
        for kit in kits.iter().filter(|k| !k.targets.is_empty()) {
            assert_eq!(TARGETS.iter().filter(|t| t.kit() == kit.name).count(), 1, "kit `{}`", kit.name);
        }
        let mut dirs: Vec<&str> = TARGETS.iter().map(|t| t.dir()).collect();
        dirs.sort();
        dirs.dedup();
        assert_eq!(dirs.len(), TARGETS.len(), "each target has its own folder");
    }
}
