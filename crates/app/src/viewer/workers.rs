//! The threads beside the window: the file watcher, which rebuilds the
//! open file when it (or a file it imports) changes, the analyzer, which
//! checks the text pane's typing, and an adopt's dry run. They answer
//! through [`Event`]s.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use rhizo_app::adopt::{Finished, Target};
use rhizo_app::project::Scan;
use rhizo_app::textpane::{self, Spans};
use rhizo_app::{Built, Engine, Failed};
use rhizo_lang::{Diagnostic, Editor};
use winit::event_loop::EventLoopProxy;

/// A build of the watched file. `generation` tells builds of a file that
/// has since been closed apart from current ones.
pub struct Reload {
    pub generation: u64,
    pub result: Result<(Box<Built>, Duration), Failed>,
}

/// The file the watcher follows, its generation (bumped on every switch)
/// and the theme to draw it in (`None`: the document's own).
pub type Watched = Arc<Mutex<(u64, Option<PathBuf>, Option<String>)>>;

/// The pane's text, to be checked off the UI thread (parsing a 5,000-node
/// file takes a few hundred milliseconds).
pub struct Request {
    pub generation: u64,
    pub version: u64,
    pub text: String,
    /// Also open an editor on it, to save it, if it has no errors.
    pub save: bool,
}

/// The answer to a [`Request`].
pub struct Analyzed {
    pub generation: u64,
    pub version: u64,
    pub diagnostics: Vec<Diagnostic>,
    pub spans: Spans,
    pub editor: Option<Editor>,
}

/// What the background threads send the window.
pub enum Event {
    Reload(Reload),
    Analyzed(Box<Analyzed>),
    Scan(Box<Scan>),
    Adopt(AdoptUpdate),
    /// A menu bar item was chosen.
    #[cfg(target_os = "macos")]
    Menu(muda::MenuId),
}

/// What an adopt's dry run says: how far it is, then its result.
pub enum AdoptUpdate {
    Step(String),
    Done(Result<(String, String, Vec<String>, bool), Finished>),
}

fn path_and<'a>(path: &'a PathBuf, deps: &'a [PathBuf]) -> impl Iterator<Item = &'a PathBuf> {
    std::iter::once(path).chain(deps)
}

/// Polls the watched file and rebuilds off the UI thread. A file that does
/// not exist yet is built as soon as it appears. Rebuilds stay close to the
/// last layout (edits, here or by an agent, leave the rest in place) until
/// the file changes or `tidy` asks for a layout from scratch.
pub fn watch(watched: Watched, tidy: Arc<AtomicBool>, proxy: EventLoopProxy<Event>, poke: Receiver<()>) {
    std::thread::spawn(move || {
        let mut engine = Engine::default();
        let mut seen = None;
        let mut last: Vec<Option<SystemTime>> = Vec::new();
        // Files the open one imports: a change to any of them rebuilds it.
        let mut deps: Vec<PathBuf> = Vec::new();
        let mtime = |p: &PathBuf| std::fs::metadata(p).and_then(|m| m.modified()).ok();
        loop {
            let (generation, path, theme) = watched.lock().expect("UI alive").clone();
            if seen != Some(generation) {
                seen = Some(generation);
                last.clear();
                deps.clear();
                engine.prev = None;
            }
            let stamp: Vec<Option<SystemTime>> = path.iter().chain(&deps).map(mtime).collect();
            let exists = stamp.first().is_some_and(Option::is_some);
            let fresh = exists && tidy.swap(false, Ordering::Relaxed);
            if let (Some(path), true) = (&path, exists && (stamp != last || fresh)) {
                last = stamp;
                if fresh {
                    engine.prev = None;
                }
                let started = Instant::now();
                let result = engine.build_file(path, theme.as_deref()).map(|b| (Box::new(b), started.elapsed()));
                if let Ok((b, _)) = &result {
                    engine.prev = Some(b.diagram.clone());
                    if b.deps != deps {
                        deps = b.deps.clone();
                        last = path_and(path, &deps).map(mtime).collect();
                    }
                }
                if proxy.send_event(Event::Reload(Reload { generation, result })).is_err() {
                    return; // window closed
                }
            }
            // A save wakes us at once; otherwise poll.
            if poke.recv_timeout(Duration::from_millis(150)) == Err(RecvTimeoutError::Disconnected) {
                return;
            }
        }
    });
}

/// Checks the pane's text off the UI thread: its problems, where its
/// statements are, and (to save it) an editor on it when it parses.
pub fn analyzer(proxy: EventLoopProxy<Event>) -> Sender<Request> {
    let (tx, rx) = mpsc::channel::<Request>();
    std::thread::spawn(move || {
        while let Ok(mut r) = rx.recv() {
            // Only the newest text matters.
            while let Ok(newer) = rx.try_recv() {
                r = newer;
            }
            let (diagnostics, spans) = textpane::analyze(&r.text);
            let ok = !diagnostics.iter().any(Diagnostic::is_error);
            let editor = (r.save && ok).then(|| Editor::open(r.text));
            let a = Analyzed { generation: r.generation, version: r.version, diagnostics, spans, editor };
            if proxy.send_event(Event::Analyzed(Box::new(a))).is_err() {
                return;
            }
        }
    });
    tx
}

/// Runs adopt on `source` as a dry run (shape checks and L1-adopt, nothing
/// written) off the UI thread, reporting each step. `cancel` kills its `tofu`;
/// `allow` are providers the person allowed for this run.
pub fn adopt(
    source: String,
    target: Target,
    allow: Vec<String>,
    cancel: Arc<AtomicBool>,
    proxy: EventLoopProxy<Event>,
) {
    let (tx, rx) = mpsc::channel::<String>();
    let steps = proxy.clone();
    std::thread::spawn(move || {
        // Ends when the run drops its sender.
        for s in rx {
            let _ = steps.send_event(Event::Adopt(AdoptUpdate::Step(s)));
        }
    });
    std::thread::spawn(move || {
        let opts = rhizo_iac::Options {
            cancel: Some(cancel),
            progress: Some(tx),
            allow_providers: allow,
            ..Default::default()
        };
        let (ids, all) = match target {
            Target::Nodes(ids) => (ids, false),
            Target::All => (Vec::new(), true),
        };
        let result = match rhizo_iac::adopt(&source, &ids, all, &opts, &[]) {
            Ok(a) => Ok((a.kdl, a.report, a.nodes, a.quiet)),
            Err(rhizo_iac::Error::Cancelled) => Err(Finished::Cancelled),
            Err(e) => Err(match &e {
                rhizo_iac::Error::NotAllowed(found) => {
                    // What this run allowed and what it refused: one more run needs them all.
                    let mut allow = opts.allow_providers.clone();
                    allow.extend(found.iter().map(|(p, _)| rhizo_iac::short_provider(p).to_string()));
                    allow.sort();
                    allow.dedup();
                    Finished::Refused { why: e.to_string(), allow }
                }
                _ => Finished::Failed(e.to_string()),
            }),
        };
        drop(opts);
        let _ = proxy.send_event(Event::Adopt(AdoptUpdate::Done(result)));
    });
}
