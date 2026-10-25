//! Renders the examples on the GPU without a window and checks the pixels.
//! Skips (passes) on machines without a GPU adapter.

use std::path::Path;

use rhizo_app::chrome::{self, Mark, Problems, Status};
use rhizo_app::draw::FontCache;
use rhizo_app::vello::Scene;
use rhizo_app::vello::kurbo::{Affine, Vec2};
use rhizo_app::{Built, Engine, render_offscreen, render_scene_offscreen, tween};
use rhizo_lang::Syntax;
use rhizo_render::Fonts;

fn check(name: &str) {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    let built = Engine::default().build_file(&path, None).expect("example builds");
    let Some((w, h, px)) = render_offscreen(&built, 1.0) else {
        eprintln!("no GPU adapter, skipping {name}");
        return;
    };
    assert_eq!(px.len(), (w * h * 4) as usize);
    let bg = built.background;
    let at = |x: u32, y: u32| &px[((y * w + x) * 4) as usize..][..3];
    assert_eq!(at(1, 1), [bg.r, bg.g, bg.b], "corner is the theme background");
    let differs = px.as_chunks::<4>().0.iter().filter(|p| p[..3] != [bg.r, bg.g, bg.b]).count();
    // Nodes, edges and text cover a real share of the canvas.
    assert!(differs * 20 > (w * h) as usize, "{name}: only {differs} drawn pixels of {}", w * h);

    // Every label is drawn inside its node: some pixels there have the text color.
    for n in built.diagram.nodes.iter().filter(|n| !n.label.is_empty()) {
        let b = built.diagram.bounds;
        let r = n.rect.translate(-b.x, -b.y);
        let close = |p: &[u8]| p.iter().zip([n.text.r, n.text.g, n.text.b]).all(|(a, b)| a.abs_diff(b) < 48);
        let (x0, y0, x1, y1) = (r.x as u32, r.y as u32, r.right() as u32, r.bottom() as u32);
        let hits = (y0..y1.min(h))
            .flat_map(|y| (x0..x1.min(w)).map(move |x| (x, y)))
            .filter(|&(x, y)| close(at(x, y)))
            .count();
        assert!(hits > 3, "{name}: no text pixels inside `{}`", n.id);
    }

    if let Ok(dir) = std::env::var("RHIZO_SNAPSHOT_DIR") {
        let out = Path::new(&dir).join(name.replace('/', "-")).with_extension("rgba");
        std::fs::write(&out, &px).unwrap();
        eprintln!("wrote {} ({w}x{h})", out.display());
    }
}

#[test]
fn product_strategy_renders() {
    check("product-strategy.rhizo");
}

#[test]
fn release_flow_renders() {
    check("release-flow.rhizo");
}

/// Typed nodes draw their kit's look (cards, rows, badges) in the window too.
#[test]
fn typed_examples_render() {
    check("shop/model.rhizo");
    check("shop/infra.rhizo");
}

fn example(name: &str, theme: Option<&str>) -> Built {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
    Engine::default().build_file(&path, theme).expect("example builds")
}

/// What the window shows: `diagram` fitted above the status bar, with
/// `extra` drawn over it in window pixels. `None` without a GPU.
fn window(
    built: &Built,
    diagram: &Scene,
    (w, h): (u32, u32),
    dpr: f64,
    extra: impl FnOnce(&mut Scene, f64),
) -> Option<Vec<u8>> {
    let view = (w as f64 - 48.0 * dpr, h as f64 - (48.0 + chrome::BAR_HEIGHT) * dpr);
    let zoom = (view.0 / built.width).min(view.1 / built.height).min(dpr);
    let offset =
        Vec2::new(w as f64 - built.width * zoom, h as f64 - chrome::BAR_HEIGHT * dpr - built.height * zoom) / 2.0;
    let mut scene = Scene::new();
    scene.append(diagram, Some(Affine::translate(offset) * Affine::scale(zoom)));
    let mut over = Scene::new();
    extra(&mut over, dpr / zoom);
    scene.append(&over, Some(Affine::translate(offset) * Affine::scale(zoom)));
    render_scene_offscreen(&scene, w, h, built.background)
}

/// Writes `px` as a PNG into `$RHIZO_SNAPSHOT_DIR`, when set.
fn snapshot(name: &str, (w, h): (u32, u32), px: &[u8]) {
    let Ok(dir) = std::env::var("RHIZO_SNAPSHOT_DIR") else { return };
    let path = Path::new(&dir).join(name);
    let mut enc = png::Encoder::new(std::fs::File::create(&path).unwrap(), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(px).unwrap();
    eprintln!("wrote {}", path.display());
}

/// Pixels in the rows `y0..y1` that are not the background.
fn drawn_rows(px: &[u8], w: u32, y0: u32, y1: u32, bg: rhizo_core::Color) -> usize {
    (y0..y1)
        .flat_map(|y| (0..w).map(move |x| ((y * w + x) * 4) as usize))
        .filter(|&i| px[i..i + 3] != [bg.r, bg.g, bg.b])
        .count()
}

#[test]
fn status_bar_hover_and_selection() {
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    for theme in ["light", "dark", "paper"] {
        let built = example("product-strategy.rhizo", Some(theme));
        let d = &built.diagram;
        let node = |id: &str| d.nodes.iter().find(|n| n.id.as_str() == id).unwrap();
        let size = (1100, 640);
        let dpr = 1.0;
        let mut glyphs = FontCache::default();
        let Some(px) = window(&built, &built.scene, size, dpr, |s, px| {
            let (sel, hover) = (node("customers"), node("risks"));
            chrome::emphasis(s, d, &built.fonts, &mut glyphs, px, Some(Mark::Node(sel)), Some(Mark::Node(hover)));
        }) else {
            return eprintln!("no GPU adapter, skipping");
        };
        // The bar goes over the rendered window.
        let mut bar = Scene::new();
        let st = Status {
            file: "product-strategy.rhizo".into(),
            nodes: built.nodes,
            edges: d.edges.len(),
            zoom: 0.87,
            selection: Some(chrome::describe_node("customers", "Customers")),
            saved: 1.0,
            project: (0, 0),
            job: None,
            hints: Vec::new(),
        };
        let win = Vec2::new(size.0 as f64, size.1 as f64);
        chrome::status_bar(&mut bar, &ui, &mut glyphs, win, dpr, built.background, &st);
        let mut scene = Scene::new();
        scene.append(&built.scene, Some(Affine::translate((0.0, -10_000.0)))); // keep fonts warm, off-canvas
        scene.append(&bar, None);
        let bar_px = render_scene_offscreen(&scene, size.0, size.1, built.background).unwrap();
        let bar_top = size.1 - chrome::BAR_HEIGHT as u32;
        assert!(drawn_rows(&bar_px, size.0, bar_top, size.1, built.background) > size.0 as usize * 20, "{theme}: bar");
        assert_eq!(drawn_rows(&bar_px, size.0, 0, bar_top - 2, built.background), 0, "{theme}: only the bar");
        // Composite for looking at it: the bar's rows over the diagram.
        let mut both = px.clone();
        let from = ((bar_top - 1) * size.0 * 4) as usize;
        both[from..].copy_from_slice(&bar_px[from..]);
        snapshot(&format!("viewer-{theme}.png"), size, &both);
    }
}

#[test]
fn problems_banner_shows_over_the_last_good_diagram() {
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/release-flow.rhizo"))
        .unwrap();
    let broken = src.replace("shape=diamond", "shap=diamond").replace("line=dashed", "line=dashd");
    let failed = Engine::default().build("release-flow.rhizo", &broken, Syntax::Kdl, None).err().expect("errors");
    assert_eq!(failed.problems.errors, 2);
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    for theme in ["light", "dark"] {
        let built = example("release-flow.rhizo", Some(theme));
        let size = (1000, 700);
        // Also shows a selected edge (the "yes" branch) and a hovered one.
        let d = &built.diagram;
        let link = |to: &str| d.edges.iter().find(|e| e.from.as_str() == "green" && e.to.as_str() == to).unwrap();
        let mut glyphs = FontCache::default();
        let Some(mut px) = window(&built, &built.scene, size, 1.0, |s, px| {
            let (sel, hover) = (Mark::Edge(link("publish-crates")), Mark::Edge(link("open-incident")));
            chrome::emphasis(s, d, &built.fonts, &mut glyphs, px, Some(sel), Some(hover));
        }) else {
            return eprintln!("no GPU adapter, skipping");
        };
        let mut s = Scene::new();
        chrome::banner(
            &mut s,
            &ui,
            &mut glyphs,
            Vec2::new(size.0 as f64, size.1 as f64),
            1.0,
            built.background,
            &failed.problems,
        );
        let banner = render_scene_offscreen(&s, size.0, size.1, built.background).unwrap();
        // Reddish pixels (the icon) in the top band, nothing below it.
        let red = banner[..(60 * size.0 * 4) as usize]
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[0] > 200 && p[1] < 100 && p[2] < 100)
            .count();
        assert!(red > 50, "{theme}: banner icon, {red} red pixels");
        assert_eq!(drawn_rows(&banner, size.0, 70, size.1, built.background), 0);
        // Composite for looking at it.
        let to = (64 * size.0 * 4) as usize;
        px[..to].copy_from_slice(&banner[..to]);
        snapshot(&format!("banner-{theme}.png"), size, &px);
    }
    // Warnings only: amber, and the count of the rest.
    let warn = Problems { errors: 0, warnings: 3, message: "unused style `risk`".into(), help: None, at: Some((4, 1)) };
    assert_eq!(warn.more().as_deref(), Some("+2 warnings"));
}

#[test]
fn relayout_frames_render() {
    let src =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/product-strategy.rhizo"))
            .unwrap();
    let mut engine = Engine::default();
    let old = engine.build("a", &src, Syntax::Kdl, None).unwrap();
    let grown = src.replace("- \"Native speed\"", "- \"Native speed\" { - \"GPU canvas\"; - \"Instant reload\" }");
    let new = engine.build("b", &grown, Syntax::Kdl, None).unwrap();
    let shift = rhizo_layout::stable::anchor_shift(&old.diagram, &new.diagram, &[]);
    for (i, t) in [0.0, 0.5, 1.0].into_iter().enumerate() {
        let f = tween::frame(&old.diagram, &new.diagram, shift, tween::ease_out(t));
        let scene = rhizo_app::draw::scene(&f, &new.fonts, &mut FontCache::default());
        let size = (1100, 700);
        let Some(px) = window(&new, &scene, size, 1.0, |_, _| {}) else {
            return eprintln!("no GPU adapter, skipping");
        };
        assert!(drawn_rows(&px, size.0, 0, size.1, new.background) > 20_000);
        snapshot(&format!("relayout-{i}.png"), size, &px);
    }
}

/// "Add child" on a mindmap and a layered diagram, drawn before, after a
/// layout from scratch and after a stable one, all in the old drawing's
/// place (held still the way the window holds it). The stable one leaves
/// what the edit did not touch where it was.
#[test]
fn stable_relayout_renders() {
    use rhizo_core::NodeId;
    use rhizo_layout::stable::{anchor_shift, anchors};
    for (name, parent) in [("playground/01-mindmap.rhizo", "kyoto"), ("release-flow.rhizo", "build-artifacts")] {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples").join(name);
        let src = std::fs::read_to_string(&path).unwrap();
        let mut ed = rhizo_lang::Editor::open(src.clone());
        let made = ed.add_child(&NodeId::new(parent), "New node").unwrap();
        let mut engine = Engine::default();
        let old = engine.build(name, &src, Syntax::Kdl, None).unwrap();
        let scratch = engine.build(name, ed.source(), Syntax::Kdl, None).unwrap();
        engine.prev = Some(old.diagram.clone());
        let stable = engine.build(name, ed.source(), Syntax::Kdl, None).unwrap();
        let prefer = anchors(&old.document, &stable.document, Some(&made));
        let shifts = [
            rhizo_core::Point::new(0.0, 0.0),
            anchor_shift(&old.diagram, &scratch.diagram, &prefer),
            anchor_shift(&old.diagram, &stable.diagram, &prefer),
        ];
        let states = [&old, &scratch, &stable];
        let canvas = states
            .iter()
            .zip(&shifts)
            .map(|(b, s)| b.diagram.bounds.translate(-s.x, -s.y))
            .reduce(|a, b| a.union(&b))
            .unwrap();
        // Unchanged nodes, mean distance from where they were.
        let moved = |b: &Built, s: rhizo_core::Point| {
            let d: Vec<f32> = b
                .diagram
                .nodes
                .iter()
                .filter_map(|n| {
                    let o = old.diagram.nodes.iter().find(|o| o.id == n.id)?;
                    let (p, q) = (o.rect.center(), n.rect.center());
                    Some(((q.x - s.x - p.x).powi(2) + (q.y - s.y - p.y).powi(2)).sqrt())
                })
                .collect();
            d.iter().sum::<f32>() / d.len() as f32
        };
        let (fresh, kept) = (moved(&scratch, shifts[1]), moved(&stable, shifts[2]));
        eprintln!("{name}: unchanged nodes moved {fresh:.1} px from scratch, {kept:.1} px stable");
        let stem = Path::new(name).file_stem().unwrap().to_string_lossy().to_string();
        let scale = 1.5;
        let size = ((canvas.width as f64 * scale).ceil() as u32, (canvas.height as f64 * scale).ceil() as u32);
        for (i, (b, s)) in states.iter().zip(&shifts).enumerate() {
            let o = b.diagram.bounds;
            let at = Vec2::new((o.x - s.x - canvas.x) as f64, (o.y - s.y - canvas.y) as f64);
            let mut scene = Scene::new();
            scene.append(&b.scene, Some(Affine::scale(scale) * Affine::translate(at)));
            let Some(px) = render_scene_offscreen(&scene, size.0, size.1, b.background) else {
                return eprintln!("no GPU adapter, skipping");
            };
            let what = ["0-before", "1-from-scratch", "2-stable"][i];
            snapshot(&format!("add-child-{stem}-{what}.png"), size, &px);
        }
        assert!(kept < 1.0, "{name}: the stable layout keeps the rest in place");
    }
}

#[test]
fn welcome_screen_renders() {
    use rhizo_app::welcome::{self, Welcome};
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let recent: Vec<std::path::PathBuf> =
        ["product-strategy.rhizo", "release-flow.rhizo"].iter().map(|n| Path::new("/Users/me/notes").join(n)).collect();
    let missing = Path::new("/Users/me/notes/agent-plan.rhizo");
    let dpr = 2.0;
    let size = (1200, 800);
    for (theme, waiting) in [("light", false), ("dark", false), ("light", true), ("dark", true)] {
        let bg = rhizo_core::Theme::by_name(theme).unwrap().background;
        let w = Welcome { missing: waiting.then_some(missing), recent: &recent };
        let mut s = Scene::new();
        let win = Vec2::new(size.0 as f64 * dpr, size.1 as f64 * dpr);
        let rects = welcome::draw(&mut s, &ui, &mut FontCache::default(), win, dpr, bg, &w, Some(4), Some(1));
        assert_eq!(rects.len(), w.items().len());
        assert!(rects.iter().all(|r| r.x0 >= 0.0 && r.x1 <= win.x && r.y1 <= win.y), "{theme}: items on screen");
        if !waiting {
            let mut glyphs = FontCache::default();
            chrome::toast(&mut s, &ui, &mut glyphs, win, dpr, bg, "Exported ~/notes/product-strategy.png", 1.0);
        }
        let (pw, ph) = (win.x as u32, win.y as u32);
        let Some(px) = render_scene_offscreen(&s, pw, ph, bg) else {
            return eprintln!("no GPU adapter, skipping");
        };
        assert!(drawn_rows(&px, pw, 0, ph, bg) > 50_000, "{theme}: something drawn");
        let name = if waiting { format!("welcome-missing-{theme}.png") } else { format!("welcome-{theme}.png") };
        snapshot(&name, (pw, ph), &px);
    }
}

#[test]
fn project_references_and_their_problems_render() {
    let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop");
    let dir = std::env::temp_dir().join(format!("rhizo-offscreen-shop-{}", std::process::id()));
    std::fs::create_dir_all(dir.join("flows")).unwrap();
    for f in ["model.rhizo", "api.rhizo"] {
        std::fs::copy(shop.join(f), dir.join(f)).unwrap();
    }
    let flow = std::fs::read_to_string(shop.join("flows/checkout.rhizo")).unwrap();
    // One typo in a reference: it still draws, and the banner says what is wrong.
    let flow =
        flow.replace("edge api:place-order confirm", "edge api:place-order confirm\n    edge confirm model:ordr");
    std::fs::write(dir.join("flows/checkout.rhizo"), flow).unwrap();
    let built = Engine::default().build_file(&dir.join("flows/checkout.rhizo"), None).expect("draws");
    let problems = built.warnings.clone().expect("the typo");
    assert!(problems.message.contains("`model:ordr`"), "{}", problems.message);
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let size = (1200, 520);
    let mut glyphs = FontCache::default();
    let Some(mut px) = window(&built, &built.scene, size, 1.0, |_, _| {}) else {
        return eprintln!("no GPU adapter, skipping");
    };
    let mut s = Scene::new();
    chrome::banner(&mut s, &ui, &mut glyphs, Vec2::new(size.0 as f64, size.1 as f64), 1.0, built.background, &problems);
    let banner = render_scene_offscreen(&s, size.0, size.1, built.background).unwrap();
    let to = (64 * size.0 * 4) as usize;
    px[..to].copy_from_slice(&banner[..to]);
    snapshot("project-references.png", size, &px);
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn text_pane_renders_beside_the_canvas() {
    use rhizo_app::textpane::{self, Extras, Pane, Statement};
    use rhizo_app::vello::kurbo::Rect;
    let src = std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/release-flow.rhizo"))
        .unwrap();
    // Typing in progress: one typo, so the canvas keeps the last good diagram.
    let typed = src.replace("shape=hexagon", "shap=hexagon");
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let mono = Fonts::with_database((*ui.database()).clone(), textpane::MONO_STACK);
    let (diagnostics, spans) = textpane::analyze(&typed);
    let problems = Problems::new(&typed, &diagnostics).expect("the typo");
    assert_eq!(spans.problems.len(), 1);
    let dpr = 2.0;
    let (w, h) = (1280.0 * dpr, 760.0 * dpr);
    for theme in ["light", "dark"] {
        let built = example("release-flow.rhizo", Some(theme));
        let d = &built.diagram;
        let mut pane = Pane::default();
        (pane.open, pane.focused) = (true, true);
        pane.reset(&typed);
        pane.spans = spans.clone();
        // The node selected on the canvas: the caret sits on its statement.
        let statement = pane.spans.of(&Statement::Node("green".into())).expect("its statement");
        pane.buffer.set_caret(statement.start, false);
        pane.refresh();
        let bar = chrome::BAR_HEIGHT * dpr;
        let rect = Rect::new(0.0, 0.0, 500.0 * dpr, h - bar);

        // The diagram fitted into the canvas right of the pane.
        let canvas = Rect::new(rect.x1, 0.0, w, h - bar);
        let m = 24.0 * dpr;
        let zoom = ((canvas.width() - 2.0 * m) / built.width).min((canvas.height() - 2.0 * m) / built.height).min(dpr);
        let offset = Vec2::new(
            canvas.x0 + (canvas.width() - built.width * zoom) / 2.0,
            (canvas.height() - built.height * zoom) / 2.0,
        );
        let view = Affine::translate(offset) * Affine::scale(zoom);
        let mut glyphs = FontCache::default();
        let mut scene = Scene::new();
        scene.append(&built.scene, Some(view));
        let mut over = Scene::new();
        let green = d.nodes.iter().find(|n| n.id.as_str() == "green").unwrap();
        chrome::emphasis(&mut over, d, &built.fonts, &mut glyphs, dpr / zoom, Some(Mark::Node(green)), None);
        scene.append(&over, Some(view));

        let mut s = Scene::new();
        let extras = Extras { statement: Some(statement.clone()), note: None };
        textpane::draw(&mut s, &pane, &mono, &ui, &mut glyphs, rect, dpr, built.background, &extras);
        let st = Status {
            file: "release-flow.rhizo".into(),
            nodes: built.nodes,
            edges: d.edges.len(),
            zoom: zoom / dpr,
            selection: Some(chrome::describe_node("green", "All green?")),
            saved: 0.0,
            project: (0, 0),
            job: None,
            hints: Vec::new(),
        };
        chrome::status_bar(&mut s, &ui, &mut glyphs, Vec2::new(w, h), dpr, built.background, &st);
        let mut banner = Scene::new();
        chrome::banner(&mut banner, &ui, &mut glyphs, Vec2::new(canvas.width(), h), dpr, built.background, &problems);
        s.append(&banner, Some(Affine::translate((canvas.x0, 0.0))));
        scene.append(&s, None);

        let (pw, ph) = (w as u32, h as u32);
        let Some(px) = render_scene_offscreen(&scene, pw, ph, built.background) else {
            return eprintln!("no GPU adapter, skipping");
        };
        let count = |x0: f64, x1: f64, f: &dyn Fn(&[u8]) -> bool| {
            (0..(rect.y1 as u32))
                .flat_map(|y| ((x0 as u32)..(x1 as u32)).map(move |x| ((y * pw + x) * 4) as usize))
                .filter(|&i| f(&px[i..i + 3]))
                .count()
        };
        // The wavy underline and the line number of the typo are red.
        let red = count(rect.x0, rect.x1, &|p| p[0] > 200 && p[1] < 110 && p[2] < 110);
        assert!(red > 40, "{theme}: error underline, {red} red pixels");
        // The caret and the statement's bar are indigo.
        let accent = count(rect.x0, rect.x1, &|p| p[2] > 180 && p[0] < 140 && p[1] < 140);
        assert!(accent > 100, "{theme}: caret and statement bar, {accent} accent pixels");
        let bg = built.background;
        let text = count(rect.x0, rect.x1, &|p| p.iter().zip([bg.r, bg.g, bg.b]).any(|(a, b)| a.abs_diff(b) > 60));
        assert!(text > 20_000, "{theme}: text drawn, {text} pixels");
        snapshot(&format!("textpane-{theme}.png"), (pw, ph), &px);
    }
}

/// A copy of the shop project with two mistakes in it.
fn broken_shop(name: &str) -> std::path::PathBuf {
    let shop = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/shop");
    let dir = std::env::temp_dir().join(format!("rhizo-offscreen-{name}-{}", std::process::id())).join("shop");
    let _ = std::fs::remove_dir_all(&dir);
    for rel in rhizo_project::list(&shop) {
        let to = dir.join(&rel);
        std::fs::create_dir_all(to.parent().unwrap()).unwrap();
        let mut text = std::fs::read_to_string(shop.join(&rel)).unwrap();
        if rel == "flows/checkout.rhizo" {
            text = text
                .replace("edge api:place-order confirm", "edge api:place-order confirm\n    edge confirm model:ordr");
        }
        if rel == "api.rhizo" {
            text = text.replace(r#"method="get""#, r#"method="fetch""#);
        }
        std::fs::write(to, text).unwrap();
    }
    dir
}

#[test]
fn project_sidebar_and_problems_render() {
    use rhizo_app::palette::{self, Entry, List, Selection};
    use rhizo_app::project::{Opened, Scan};
    use rhizo_app::sidebar::{self, Glyph, Layout, Marks, Model, Sidebar};
    use rhizo_app::vello::kurbo::Rect;
    let dir = broken_shop("sidebar");
    let scan = Scan::read(1, &Opened::Folder(dir.clone()), &rhizo_kit::Kits::builtin());
    assert_eq!(scan.counts(None), (2, 0), "{:#?}", scan.problems());
    let model = Model::new(&scan);
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let dpr = 2.0;
    let (w, h) = (1280.0 * dpr, 760.0 * dpr);
    let bar = chrome::BAR_HEIGHT * dpr;
    for theme in ["light", "dark"] {
        let built = Engine::default().build_file(&dir.join("model.rhizo"), Some(theme)).expect("draws");
        let d = &built.diagram;
        let mut state = Sidebar::default();
        state.expanded.insert("model.rhizo".into());
        state.expanded.insert("intent.rhizo".into());
        let side = Rect::new(0.0, 0.0, sidebar::WIDTH * dpr, h - bar);
        let layout = Layout::new(&model, &mut state, side, dpr);

        // The diagram fitted into the canvas right of the sidebar, "Order" selected.
        let canvas = Rect::new(side.x1, 0.0, w, h - bar);
        let m = 24.0 * dpr;
        let zoom = ((canvas.width() - 2.0 * m) / built.width).min((canvas.height() - 2.0 * m) / built.height).min(dpr);
        let offset = Vec2::new(
            canvas.x0 + (canvas.width() - built.width * zoom) / 2.0,
            (canvas.height() - built.height * zoom) / 2.0,
        );
        let view = Affine::translate(offset) * Affine::scale(zoom);
        let mut glyphs = FontCache::default();
        let mut base = Scene::new();
        base.append(&built.scene, Some(view));
        let order = d.nodes.iter().find(|n| n.id.as_str() == "order").unwrap();
        let mut over = Scene::new();
        chrome::emphasis(&mut over, d, &built.fonts, &mut glyphs, dpr / zoom, Some(Mark::Node(order)), None);
        base.append(&over, Some(view));

        let marks = Marks {
            current: Some("model.rhizo"),
            selected: Some("order"),
            hover: layout.rows.get(3).map(|r| r.1),
            drop: None,
            hover_footer: false,
        };
        let mut s = Scene::new();
        sidebar::draw(&mut s, &ui, &mut glyphs, &model, &layout, dpr, built.background, &marks);
        let used = scan.used_in("model.rhizo", &"order".into());
        assert_eq!(used, 2);
        let st = Status {
            file: "model.rhizo".into(),
            nodes: built.nodes,
            edges: d.edges.len(),
            zoom: zoom / dpr,
            selection: Some(format!(
                "{} · used in {}",
                chrome::describe_node("order", "Order"),
                chrome::plural(used, "file")
            )),
            saved: 0.0,
            project: scan.counts(None),
            job: None,
            hints: Vec::new(),
        };
        chrome::status_bar(&mut s, &ui, &mut glyphs, Vec2::new(w, h), dpr, built.background, &st);
        base.append(&s, None);

        let (pw, ph) = (w as u32, h as u32);
        let Some(px) = render_scene_offscreen(&base, pw, ph, built.background) else {
            return eprintln!("no GPU adapter, skipping");
        };
        // The sidebar is drawn, the error pills and footer dot are red.
        let red = (0..ph)
            .flat_map(|y| (0..side.x1 as u32).map(move |x| ((y * pw + x) * 4) as usize))
            .filter(|&i| px[i] > 200 && px[i + 1] < 110 && px[i + 2] < 110)
            .count();
        assert!(red > 30, "{theme}: error marks in the sidebar, {red} red pixels");
        assert!(drawn_rows(&px, pw, 0, ph, built.background) > 100_000);
        snapshot(&format!("sidebar-{theme}.png"), (pw, ph), &px);

        // The Problems list over the same window.
        let problems = scan.problems();
        let entries: Vec<Entry> = problems
            .iter()
            .map(|p| Entry {
                name: p.message.clone(),
                detail: p.place(),
                glyph: Some(if p.error { Glyph::Error } else { Glyph::Warning }),
            })
            .collect();
        let rows = palette::rows(List::Problems, "", Selection::Node, &entries);
        assert_eq!(rows.len(), 2);
        let mut pal = Scene::new();
        let rects = palette::draw(
            &mut pal,
            &ui,
            &mut glyphs,
            Vec2::new(w, h),
            dpr,
            built.background,
            (List::Problems, "", 0),
            &rows,
        );
        assert_eq!(rects.len(), 2);
        base.append(&pal, None);
        let px = render_scene_offscreen(&base, pw, ph, built.background).unwrap();
        snapshot(&format!("problems-{theme}.png"), (pw, ph), &px);
    }
    std::fs::remove_dir_all(dir.parent().unwrap()).unwrap();
}

/// The inspector beside the canvas: a POST operation's schema form, and an
/// untyped mindmap node with its "Set type…" button.
#[test]
fn inspector_renders_typed_and_untyped_nodes() {
    use rhizo_app::inspector::{self, Model, Place};
    use rhizo_app::vello::kurbo::Rect;
    let kits = rhizo_kit::Kits::builtin();
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let dpr = 2.0;
    let (w, h) = (1280.0 * dpr, 760.0 * dpr);
    let bar = chrome::BAR_HEIGHT * dpr;
    let cases = [("shop/api.rhizo", "place-order", "light"), ("product-strategy.rhizo", "customers", "dark")];
    for (file, id, theme) in cases {
        let built = example(file, Some(theme));
        let d = &built.diagram;
        let node = &built.document.nodes[&rhizo_core::NodeId::from(id)];
        let def = node.ty.as_ref().and_then(|t| kits.find_type(t.as_str()));
        let model = Model {
            label: node.label.clone(),
            id: id.into(),
            ty: node.ty.as_ref().map(|t| {
                let kit = kits.owner(t.as_str()).map(|(k, _)| k.name.clone());
                (t.to_string(), def.map(|d| (d.label.clone(), kit.unwrap_or_default())))
            }),
            fields: inspector::fields(def, &node.props, None),
            place: if def.is_some() { Place::UsedIn(vec!["flows/checkout.rhizo".into()]) } else { Place::Alone },
            editable: true,
            adopt: None,
            ..Model::default()
        };
        let panel = Rect::new(w - inspector::WIDTH * dpr, 0.0, w, h - bar);
        let canvas = Rect::new(0.0, 0.0, panel.x0, h - bar);
        let m = 24.0 * dpr;
        let zoom = ((canvas.width() - 2.0 * m) / built.width).min((canvas.height() - 2.0 * m) / built.height).min(dpr);
        let offset =
            Vec2::new((canvas.width() - built.width * zoom) / 2.0, (canvas.height() - built.height * zoom) / 2.0);
        let view = Affine::translate(offset) * Affine::scale(zoom);
        let mut glyphs = FontCache::default();
        let mut scene = Scene::new();
        scene.append(&built.scene, Some(view));
        let sel = d.nodes.iter().find(|n| n.id.as_str() == id).unwrap();
        let mut over = Scene::new();
        chrome::emphasis(&mut over, d, &built.fonts, &mut glyphs, dpr / zoom, Some(Mark::Node(sel)), None);
        scene.append(&over, Some(view));
        let mut s = Scene::new();
        let hits =
            inspector::draw(&mut s, &ui, &mut glyphs, Some(&model), None, None, panel, dpr, built.background, 0.0).0;
        assert!(hits.iter().any(|(_, h)| *h == inspector::Hit::Type), "{file}: the type row is clickable");
        assert!(hits.iter().all(|(r, _)| r.x0 >= panel.x0 && r.y1 <= panel.y1), "{file}: hits inside the panel");
        let st = Status {
            file: file.into(),
            nodes: built.nodes,
            edges: d.edges.len(),
            zoom: zoom / dpr,
            selection: Some(chrome::describe_node(id, &node.label)),
            saved: 0.0,
            project: (0, 0),
            job: None,
            hints: Vec::new(),
        };
        chrome::status_bar(&mut s, &ui, &mut glyphs, Vec2::new(w, h), dpr, built.background, &st);
        scene.append(&s, None);
        let (pw, ph) = (w as u32, h as u32);
        let Some(px) = render_scene_offscreen(&scene, pw, ph, built.background) else {
            return eprintln!("no GPU adapter, skipping");
        };
        let bg = built.background;
        let text = (0..panel.y1 as u32)
            .flat_map(|y| ((panel.x0 as u32)..pw).map(move |x| ((y * pw + x) * 4) as usize))
            .filter(|&i| px[i..i + 3].iter().zip([bg.r, bg.g, bg.b]).any(|(a, b)| a.abs_diff(b) > 60))
            .count();
        assert!(text > 5_000, "{file}: the panel is drawn, {text} pixels");
        snapshot(&format!("inspector-{}.png", id), (pw, ph), &px);
    }
}

/// Adopting from the viewer: the inspector's "Adopt…" button on a node of
/// imported code (with the job in the status bar), a refusal with its
/// reason, and the report dialog that waits for Adopt.
#[test]
fn adopt_button_and_report_render() {
    use rhizo_app::adopt::{self, Button, Review, Target};
    use rhizo_app::inspector::{self, Adoption, Model, Place};
    use rhizo_app::vello::kurbo::Rect;
    let kits = rhizo_kit::Kits::builtin();
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let mono = Fonts::system(rhizo_app::textpane::MONO_STACK);
    let dpr = 2.0;
    let (w, h) = (1280.0 * dpr, 760.0 * dpr);
    let bar = chrome::BAR_HEIGHT * dpr;
    let built = example("shop/infra.rhizo", Some("light"));
    let def = kits.find_type("infra.queue");
    let mut props = rhizo_core::Props::new();
    props.insert("visibility-timeout-s".into(), serde_json::json!(30));
    let model = |adopt| Model {
        label: "jobs".into(),
        id: "aws_sqs_queue.jobs".into(),
        ty: Some(("infra.queue".into(), Some(("Queue".into(), "infra".into())))),
        fields: inspector::fields(def, &props, None),
        place: Place::Alone,
        editable: true,
        adopt: Some(adopt),
        ..Model::default()
    };
    let refused = "cannot adopt `aws_lambda_function.worker` from shape resources (`aws_lambda_function`) yet: rhizo's code for an infra.function is not one";
    let report = "rhizo adopt\n  adopt aws_sqs_queue.jobs (infra.queue) from resources (aws_sqs_queue): rhizo's code at module.jobs\n    kept from its code (names, places and tags of what exists): name\nL1-adopt (offline: both codes planned with mocked providers against an empty state): every original instance is accounted for: 1 moved, 3 at their own address\n  moves aws_sqs_queue.jobs -> module.jobs.aws_sqs_queue.this[0]\n  adopting changes module.jobs.aws_sqs_queue.this[0] (from aws_sqs_queue.jobs): fifo_queue: null → false\n  adopting changes module.jobs.aws_sqs_queue.this[0] (from aws_sqs_queue.jobs): tags: null → {\"rhizo:node\":\"jobs\"}\n  creates: none\nLive (a person's step): `tofu plan` against the real state must show only these moves, these in-place updates and these creates; any replace or destroy is a stop. Only a live plan shows whether a provider replaces a resource for a changed attribute.\n";
    let review = Review {
        target: Target::Nodes(vec!["aws_sqs_queue.jobs".into()]),
        path: "/x/raw-aws.rhizo".into(),
        source: String::new(),
        kdl: String::new(),
        report: report.into(),
        nodes: vec!["aws_sqs_queue.jobs".into()],
        scroll: 0,
        allow: vec![],
    };
    for (name, adoption, dialog) in [
        ("adopt-inspector", Adoption::Ready, false),
        ("adopt-refused", Adoption::Refused(refused.into()), false),
        ("adopt-report", Adoption::Running, true),
    ] {
        let panel = Rect::new(w - inspector::WIDTH * dpr, 0.0, w, h - bar);
        let mut glyphs = FontCache::default();
        let mut s = Scene::new();
        let hits = inspector::draw(
            &mut s,
            &ui,
            &mut glyphs,
            Some(&model(adoption.clone())),
            None,
            None,
            panel,
            dpr,
            built.background,
            0.0,
        )
        .0;
        let button = hits.iter().any(|(_, h)| *h == inspector::Hit::Adopt);
        assert_eq!(button, adoption == Adoption::Ready, "{name}: Adopt… only when it can be adopted");
        let shown = |(r, h): &(Rect, inspector::Hit)| *h != inspector::Hit::Adopt || r.y1 <= panel.y1 / 2.0;
        assert!(hits.iter().all(shown), "{name}: Adopt… shows near the top, above a long form");
        let st = Status {
            file: "raw-aws.rhizo".into(),
            nodes: built.nodes,
            edges: built.diagram.edges.len(),
            zoom: 0.5,
            selection: Some(chrome::describe_node("aws_sqs_queue.jobs", "jobs")),
            saved: 0.0,
            project: (0, 0),
            job: (adoption == Adoption::Running)
                .then(|| "Adopting `aws_sqs_queue.jobs`… planning the original code".into()),
            hints: Vec::new(),
        };
        chrome::status_bar(&mut s, &ui, &mut glyphs, Vec2::new(w, h), dpr, built.background, &st);
        if dialog {
            let rects = adopt::draw(
                &mut s,
                &ui,
                &mono,
                &mut glyphs,
                Vec2::new(w, h),
                dpr,
                built.background,
                &review,
                Some(Button::Adopt),
            );
            let names: Vec<Button> = rects.iter().map(|(_, b)| *b).collect();
            assert_eq!(names, [Button::Adopt, Button::Cancel]);
            assert!(rects.iter().all(|(r, _)| r.x1 <= w && r.y1 <= h && r.x0 >= 0.0), "buttons inside the window");
            let (lines, _, total) = adopt::visible(&review, &mono, Vec2::new(w, h), dpr);
            assert_eq!(lines.len(), total, "the whole report fits: {total} lines");
        }
        let canvas = Rect::new(0.0, 0.0, panel.x0, h - bar);
        let zoom = ((canvas.width() - 48.0 * dpr) / built.width).min((canvas.height() - 48.0 * dpr) / built.height);
        let offset =
            Vec2::new((canvas.width() - built.width * zoom) / 2.0, (canvas.height() - built.height * zoom) / 2.0);
        let mut scene = Scene::new();
        scene.append(&built.scene, Some(Affine::translate(offset) * Affine::scale(zoom)));
        scene.append(&s, None);
        let Some(px) = render_scene_offscreen(&scene, w as u32, h as u32, built.background) else {
            return eprintln!("no GPU adapter, skipping");
        };
        snapshot(&format!("{name}.png"), (w as u32, h as u32), &px);
    }
}

/// The diagram fitted into `canvas` (window pixels): its transform and zoom.
fn fit_in(built: &Built, canvas: rhizo_app::vello::kurbo::Rect, dpr: f64) -> (Affine, f64) {
    let m = 24.0 * dpr;
    let zoom = ((canvas.width() - 2.0 * m) / built.width).min((canvas.height() - 2.0 * m) / built.height).min(dpr);
    let offset = Vec2::new(
        canvas.x0 + (canvas.width() - built.width * zoom) / 2.0,
        canvas.y0 + (canvas.height() - built.height * zoom) / 2.0,
    );
    (Affine::translate(offset) * Affine::scale(zoom), zoom)
}

/// Pixels of `px` (a `w` wide image) inside `r` that `f` picks.
fn count_in(px: &[u8], w: u32, r: rhizo_app::vello::kurbo::Rect, f: impl Fn(&[u8]) -> bool) -> usize {
    (r.y0.max(0.0) as u32..r.y1 as u32)
        .flat_map(|y| (r.x0.max(0.0) as u32..r.x1 as u32).map(move |x| ((y * w + x) * 4) as usize))
        .filter(|&i| f(&px[i..i + 3]))
        .count()
}

/// The window's own UI: a context menu on a node, a tooltip, three nodes
/// selected with the selection box, an inspector field with a caret and a
/// selection, and the focus ring on the text pane.
#[test]
fn ux_chrome_renders() {
    use rhizo_app::context::{self, Item, Place};
    use rhizo_app::field::Field;
    use rhizo_app::inspector::{self, Model, Typing};
    use rhizo_app::palette::{self, MenuRow, Selection};
    use rhizo_app::textpane::{self, Extras, Pane};
    use rhizo_app::vello::kurbo::{Point, Rect};
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let accent = |p: &[u8]| p[2] > 180 && p[0] < 150 && p[1] < 150;
    let dpr = 2.0;
    let (w, h) = (1280.0 * dpr, 760.0 * dpr);
    let (pw, ph) = (w as u32, h as u32);
    let bar = chrome::BAR_HEIGHT * dpr;
    let status = |built: &Built, file: &str, selection: String| Status {
        file: file.into(),
        nodes: built.nodes,
        edges: built.diagram.edges.len(),
        zoom: 0.8,
        selection: Some(selection),
        saved: 0.0,
        project: (0, 0),
        job: None,
        hints: Vec::new(),
    };

    // A context menu on a node, and the tooltip of the inspector's type row.
    let built = example("product-strategy.rhizo", Some("light"));
    let d = &built.diagram;
    let node = d.nodes.iter().find(|n| n.id.as_str() == "customers").unwrap();
    let panel = Rect::new(w - inspector::WIDTH * dpr, 0.0, w, h - bar);
    let (view, zoom) = fit_in(&built, Rect::new(0.0, 0.0, panel.x0, h - bar), dpr);
    let to_window = |p: rhizo_core::Point| view * Point::new((p.x - d.bounds.x) as f64, (p.y - d.bounds.y) as f64);
    let mut glyphs = FontCache::default();
    let base = |glyphs: &mut FontCache, marks: &[&str]| {
        let mut scene = Scene::new();
        scene.append(&built.scene, Some(view));
        let mut over = Scene::new();
        for id in marks {
            let n = d.nodes.iter().find(|n| n.id.as_str() == *id).unwrap();
            chrome::emphasis(&mut over, d, &built.fonts, glyphs, dpr / zoom, Some(Mark::Node(n)), None);
        }
        scene.append(&over, Some(view));
        scene
    };
    let mut scene = base(&mut glyphs, &["customers"]);
    let at = to_window(node.rect.center()).to_vec2();
    let rows: Vec<MenuRow> = context::items(&Place::Node { reference: false, many: false, code: false })
        .iter()
        .map(|i| match i {
            Item::Run(a) => context::row(*a, Selection::Node),
            _ => MenuRow { name: String::new(), keys: String::new(), enabled: false },
        })
        .collect();
    let mut s = Scene::new();
    let st = status(&built, "product-strategy.rhizo", chrome::describe_node("customers", "Customers"));
    chrome::status_bar(&mut s, &ui, &mut glyphs, Vec2::new(w, h), dpr, built.background, &st);
    let rects =
        palette::draw_menu(&mut s, &ui, &mut glyphs, at, Vec2::new(w, h), dpr, built.background, &rows, Some(4));
    assert_eq!(rects.len(), rows.iter().filter(|r| r.enabled).count(), "every enabled row can be clicked");
    let menu = rects.iter().fold(rects[0].0, |a, (r, _)| a.union(*r));
    assert!(menu.x0 >= at.x - 1.0 && menu.y0 >= at.y - 1.0 && menu.y1 < h, "the menu opens at the pointer, inside");
    scene.append(&s, None);
    let Some(px) = render_scene_offscreen(&scene, pw, ph, built.background) else {
        return eprintln!("no GPU adapter, skipping");
    };
    let hovered = rects.iter().find(|(_, i)| *i == 4).unwrap().0;
    assert!(count_in(&px, pw, hovered, accent) * 2 > (hovered.area() as usize), "the hovered row is accent");
    snapshot("ux-context-menu.png", (pw, ph), &px);

    let kits = rhizo_kit::Kits::builtin();
    let model_of = |file: &str, id: &str| {
        let b = example(file, Some("light"));
        let n = b.document.nodes[&rhizo_core::NodeId::from(id)].clone();
        let def = n.ty.as_ref().and_then(|t| kits.find_type(t.as_str()));
        let ty = n.ty.as_ref().map(|t| {
            let kit = kits.owner(t.as_str()).map(|(k, _)| k.name.clone());
            (t.to_string(), def.map(|d| (d.label.clone(), kit.unwrap_or_default())))
        });
        let fields = inspector::fields(def, &n.props, None);
        Model {
            label: n.label.clone(),
            id: id.into(),
            ty,
            fields,
            place: inspector::Place::UsedIn(vec!["flows/checkout.rhizo".into()]),
            editable: true,
            adopt: None,
            ..Model::default()
        }
    };
    let model = model_of("product-strategy.rhizo", "customers");
    let mut scene = base(&mut glyphs, &["customers"]);
    let mut s = Scene::new();
    let hits = inspector::draw(
        &mut s,
        &ui,
        &mut glyphs,
        Some(&model),
        None,
        Some(inspector::Hit::InsertReference),
        panel,
        dpr,
        built.background,
        0.0,
    )
    .0;
    let ty = hits.iter().find(|(_, h)| *h == inspector::Hit::InsertReference).unwrap().0;
    let tip = inspector::tip(&model, inspector::Hit::InsertReference);
    let (text, keys) = tip.split_once("   ").unwrap_or((tip.as_str(), ""));
    assert_eq!((text, keys), ("Insert reference…", "⌘R"), "the registry's name and keys");
    let pointer = Vec2::new(ty.center().x, ty.center().y);
    chrome::tooltip(&mut s, &ui, &mut glyphs, pointer, Vec2::new(w, h), dpr, built.background, text, keys);
    chrome::status_bar(&mut s, &ui, &mut glyphs, Vec2::new(w, h), dpr, built.background, &st);
    scene.append(&s, None);
    let px = render_scene_offscreen(&scene, pw, ph, built.background).unwrap();
    let card = Rect::new(pointer.x, pointer.y + 20.0 * dpr, pointer.x + 60.0 * dpr, pointer.y + 44.0 * dpr);
    let dark = count_in(&px, pw, card, |p| p.iter().all(|&c| c < 80));
    assert!(dark * 2 > card.area() as usize, "a dark card under the pointer, {dark} dark pixels");
    snapshot("ux-tooltip.png", (pw, ph), &px);

    // Three nodes of a free canvas selected, with the selection box that picked them.
    let built = example("playground/03-canvas.rhizo", Some("light"));
    let d = &built.diagram;
    let (view, zoom) = fit_in(&built, Rect::new(0.0, 0.0, w, h - bar), dpr);
    let picked = ["login-is-slow", "cache-user-sessions", "ask-dba-about-indexes"];
    let boxes: Vec<rhizo_core::Rect> =
        picked.iter().map(|id| d.nodes.iter().find(|n| n.id.as_str() == *id).unwrap().rect).collect();
    let r = boxes.iter().fold(boxes[0], |a, b| {
        let (x, y) = (a.x.min(b.x), a.y.min(b.y));
        rhizo_core::Rect::new(x, y, a.right().max(b.right()) - x, a.bottom().max(b.bottom()) - y)
    });
    let band = rhizo_core::Rect::new(r.x - 12.0, r.y - 12.0, r.width + 24.0, r.height + 24.0);
    let mut got = rhizo_app::interact::in_rect(d, band);
    got.sort();
    let mut want: Vec<rhizo_core::NodeId> = picked.iter().map(|id| rhizo_core::NodeId::from(*id)).collect();
    want.sort();
    assert_eq!(got, want, "the box picks the three it meets");
    let mut scene = Scene::new();
    scene.append(&built.scene, Some(view));
    let mut over = Scene::new();
    for id in picked {
        let n = d.nodes.iter().find(|n| n.id.as_str() == id).unwrap();
        chrome::emphasis(&mut over, d, &built.fonts, &mut glyphs, dpr / zoom, Some(Mark::Node(n)), None);
    }
    scene.append(&over, Some(view));
    let corner = |x: f32, y: f32| view * Point::new((x - d.bounds.x) as f64, (y - d.bounds.y) as f64);
    let band_px = Rect::from_points(corner(band.x, band.y), corner(band.right(), band.bottom()));
    let mut s = Scene::new();
    chrome::selection_box(&mut s, band_px, dpr);
    chrome::status_bar(
        &mut s,
        &ui,
        &mut glyphs,
        Vec2::new(w, h),
        dpr,
        built.background,
        &status(&built, "03-canvas.rhizo", "3 selected".into()),
    );
    scene.append(&s, None);
    let px = render_scene_offscreen(&scene, pw, ph, built.background).unwrap();
    let edge = Rect::new(band_px.x0 - 2.0, band_px.y0 + 10.0, band_px.x0 + 2.0, band_px.y1 - 10.0);
    assert!(count_in(&px, pw, edge, accent) > 20, "the box's edge is drawn");
    snapshot("ux-selection.png", (pw, ph), &px);

    // An inspector field being typed in: "/orders/{id}" with "orders" selected.
    let built = example("shop/api.rhizo", Some("light"));
    let model = model_of("shop/api.rhizo", "place-order");
    let path = model.fields.iter().position(|f| f.key == "path").unwrap();
    let mut field = Field::new("/orders/{id}");
    field.buffer.anchor = 1;
    field.buffer.head = 7;
    let (view, _) = fit_in(&built, Rect::new(0.0, 0.0, panel.x0, h - bar), dpr);
    let mut scene = Scene::new();
    scene.append(&built.scene, Some(view));
    let mut s = Scene::new();
    let typing = Typing { at: inspector::Hit::Field(path), field_text: &field };
    let hits =
        inspector::draw(&mut s, &ui, &mut glyphs, Some(&model), Some(&typing), None, panel, dpr, built.background, 0.0)
            .0;
    chrome::focus_ring(&mut s, panel, dpr);
    scene.append(&s, None);
    let px = render_scene_offscreen(&scene, pw, ph, built.background).unwrap();
    let row = hits.iter().find(|(_, h)| *h == inspector::Hit::Field(path)).unwrap().0;
    let tinted = count_in(&px, pw, row, |p| p[2] as u16 > p[0] as u16 + 20 && p[2] > 200 && p[0] > 150);
    assert!(tinted > 200, "the selection is tinted, {tinted} pixels");
    snapshot("ux-inspector-field.png", (pw, ph), &px);

    // The text pane has the keys: its focus ring.
    let built = example("release-flow.rhizo", Some("light"));
    let mono = Fonts::with_database((*ui.database()).clone(), textpane::MONO_STACK);
    let mut pane = Pane::default();
    (pane.open, pane.focused) = (true, true);
    pane.reset(&built.source);
    pane.refresh();
    let rect = Rect::new(0.0, 0.0, 500.0 * dpr, h - bar);
    let (view, _) = fit_in(&built, Rect::new(rect.x1, 0.0, w, h - bar), dpr);
    let mut scene = Scene::new();
    scene.append(&built.scene, Some(view));
    let mut s = Scene::new();
    textpane::draw(
        &mut s,
        &pane,
        &mono,
        &ui,
        &mut glyphs,
        rect,
        dpr,
        built.background,
        &Extras { statement: None, note: None },
    );
    chrome::focus_ring(&mut s, rect, dpr);
    chrome::status_bar(
        &mut s,
        &ui,
        &mut glyphs,
        Vec2::new(w, h),
        dpr,
        built.background,
        &status(&built, "release-flow.rhizo", "text pane".into()),
    );
    scene.append(&s, None);
    let px = render_scene_offscreen(&scene, pw, ph, built.background).unwrap();
    let right_edge = Rect::new(rect.x1 - 4.0 * dpr, 40.0, rect.x1, rect.y1 - 40.0);
    assert!(count_in(&px, pw, right_edge, accent) > (rect.height() as usize), "the ring runs down the pane's edge");
    snapshot("ux-focus-pane.png", (pw, ph), &px);
}

/// What is selected in [`with_inspector`].
enum Selected {
    Frame(rhizo_core::FrameId),
    Edge(usize),
}

/// Renders `doc` (KDL source) in a window with the inspector open showing
/// `model`, the canvas emphasising what `mark` picks. Returns the pixels
/// and their size, `None` without a GPU.
fn with_inspector(
    src: &str,
    theme: &str,
    model: &rhizo_app::inspector::Model,
    selected: Selected,
) -> Option<(Vec<u8>, (u32, u32))> {
    use rhizo_app::inspector;
    use rhizo_app::vello::kurbo::Rect;
    let built = Engine::default().build("prod.rhizo", src, Syntax::Kdl, Some(theme)).expect("builds");
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let dpr = 2.0;
    let (w, h) = (1280.0 * dpr, 860.0 * dpr);
    let bar = chrome::BAR_HEIGHT * dpr;
    let panel = Rect::new(w - inspector::WIDTH * dpr, 0.0, w, h - bar);
    let (view, zoom) = fit_in(&built, Rect::new(0.0, 0.0, panel.x0, h - bar), dpr);
    let mut glyphs = FontCache::default();
    let mut scene = Scene::new();
    scene.append(&built.scene, Some(view));
    let mut over = Scene::new();
    let d = &built.diagram;
    let mark = match &selected {
        Selected::Frame(f) => d.frames.iter().find(|x| &x.id == f).map(Mark::Frame),
        Selected::Edge(k) => rhizo_app::interact::drawn_edge(d, &built.document, *k).map(Mark::Edge),
    };
    chrome::emphasis(&mut over, d, &built.fonts, &mut glyphs, dpr / zoom, mark, None);
    scene.append(&over, Some(view));
    let mut s = Scene::new();
    let (hits, height) =
        inspector::draw(&mut s, &ui, &mut glyphs, Some(model), None, None, panel, dpr, built.background, 0.0);
    assert!(!hits.is_empty() && height > 0.0);
    scene.append(&s, None);
    let (pw, ph) = (w as u32, h as u32);
    render_scene_offscreen(&scene, pw, ph, built.background).map(|px| (px, (pw, ph)))
}

const PRODUCTION: &str = r#"rhizo version=2
use infra version="0.1"
frame "Production" type=infra.environment {
    props cloud="aws" region="eu-central-1" budget-monthly=200 {
        azure resource-group="rg-shop"
    }
    node "Bucket" id=bucket type=infra.bucket
    node "Function" id=function type=infra.function
    edge function bucket {
        props access="write"
    }
}
"#;

/// A frame selected with its inspector (an environment: cloud, region,
/// state bucket, the cloud blocks, budget), and an edge with its kind,
/// line, arrowheads and the access the infra kit gives it.
#[test]
fn frames_and_edges_show_in_the_inspector() {
    use rhizo_app::ops;
    let kits = rhizo_kit::Kits::builtin();
    let e = rhizo_lang::Editor::open(PRODUCTION);
    let doc = e.document();
    let prod = rhizo_core::FrameId::from("production");
    let m = ops::frame_model(doc, &kits, &prod, None).unwrap();
    let keys: Vec<&str> = m.fields.iter().map(|f| f.key.as_str()).collect();
    assert!(keys.starts_with(&["cloud", "region", "state-bucket", "azure.resource-group"]), "{keys:?}");
    for theme in ["light", "dark"] {
        let Some((px, size)) = with_inspector(PRODUCTION, theme, &m, Selected::Frame(prod.clone())) else {
            return eprintln!("no GPU adapter, skipping");
        };
        snapshot(&format!("frame-inspector-{theme}.png"), size, &px);
    }
    let k = doc.edges.iter().position(|x| x.from.as_str() == "function").unwrap();
    let m = ops::edge_model(doc, k, None).unwrap();
    assert_eq!(m.fields.iter().map(|f| f.key.as_str()).collect::<Vec<_>>(), ["access", "prefix"]);
    assert_eq!(m.fields[0].value, Some(serde_json::json!("write")));
    if let Some((px, size)) = with_inspector(PRODUCTION, "light", &m, Selected::Edge(k)) {
        snapshot("edge-inspector.png", size, &px);
    }
}

/// ⌘G: two steps of a flow grouped into a frame, the edges among them inside it.
#[test]
fn grouping_into_a_frame_renders() {
    use rhizo_app::ops;
    let src = "rhizo version=1\nlayout layered\nnode \"Cart\"\nnode \"Checkout\"\nnode \"Payment\"\nnode \"Receipt\"\nedge cart checkout\nedge checkout payment\nedge payment receipt\n";
    let mut e = rhizo_lang::Editor::open(src);
    let f = e.group(&["checkout".into(), "payment".into()], "New frame").unwrap();
    let f = ops::name_new_frame(&mut e, &f, "Pay").unwrap();
    assert!(
        e.source()
            .contains("frame \"Pay\" {\n    node \"Checkout\"\n    node \"Payment\"\n    edge checkout payment\n}"),
        "{}",
        e.source()
    );
    let kits = rhizo_kit::Kits::builtin();
    let m = ops::frame_model(e.document(), &kits, &f, None).unwrap();
    let Some((px, size)) = with_inspector(e.source(), "light", &m, Selected::Frame(f)) else {
        return eprintln!("no GPU adapter, skipping");
    };
    snapshot("group-result.png", size, &px);
    if let Ok(dir) = std::env::var("RHIZO_SNAPSHOT_DIR") {
        std::fs::write(Path::new(&dir).join("group-result.rhizo"), e.source()).unwrap();
    }
}

/// The welcome footer with the Franconian rake, in both themes, the rake
/// up close, and the About dialog.
#[test]
fn the_footer_and_about_render() {
    use rhizo_app::about;
    use rhizo_app::welcome::{self, Welcome};
    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let dpr = 2.0;
    let win = Vec2::new(1200.0 * dpr, 800.0 * dpr);
    let (pw, ph) = (win.x as u32, win.y as u32);
    let recent = [Path::new("/Users/me/notes/product-strategy.rhizo").to_path_buf()];
    for theme in ["light", "dark"] {
        let bg = rhizo_core::Theme::by_name(theme).unwrap().background;
        let w = Welcome { missing: None, recent: &recent };
        let mut s = Scene::new();
        let mut glyphs = FontCache::default();
        let rects = welcome::draw(&mut s, &ui, &mut glyphs, win, dpr, bg, &w, None, None);
        let items = w.items();
        let github = rects[items.iter().position(|i| *i == welcome::Item::GitHub).unwrap()];
        assert!(github.y1 <= win.y && github.y0 > win.y * 0.85, "{theme}: the footer sits at the bottom");
        let Some(px) = render_scene_offscreen(&s, pw, ph, bg) else {
            return eprintln!("no GPU adapter, skipping");
        };
        // The rake's red is drawn in the footer.
        let red = |p: &[u8]| p[0] > 200 && p[1] < 60 && p[2] < 60;
        let footer = rhizo_app::vello::kurbo::Rect::new(0.0, win.y * 0.85, win.x, win.y);
        assert!(count_in(&px, pw, footer, red) > 40, "{theme}: the rake and the heart are red");
        snapshot(&format!("welcome-footer-{theme}.png"), (pw, ph), &px);
        // The footer rows, cropped.
        let y0 = (win.y - 130.0 * dpr) as u32;
        snapshot(&format!("welcome-footer-crop-{theme}.png"), (pw, ph - y0), &px[(y0 * pw * 4) as usize..]);

        // The About dialog over it.
        let (panel, links) = about::draw(&mut s, &ui, &mut glyphs, win, dpr, bg, Some(about::Link::GitHub));
        assert!(links.iter().any(|(r, l)| *l == about::Link::GitHub && panel.contains(r.center())));
        assert!(links.iter().all(|(_, l)| *l != about::Link::About), "About does not link to itself in the dialog");
        let px = render_scene_offscreen(&s, pw, ph, bg).unwrap();
        snapshot(&format!("about-{theme}.png"), (pw, ph), &px);
    }
    // The rake up close, on light and dark.
    let (cw, ch) = (720u32, 400u32);
    let mut s = Scene::new();
    for (i, theme) in ["light", "dark"].into_iter().enumerate() {
        let bg = rhizo_core::Theme::by_name(theme).unwrap().background;
        let x0 = i as f64 * 360.0;
        s.fill(
            rhizo_app::vello::peniko::Fill::NonZero,
            Affine::IDENTITY,
            rhizo_app::peniko_color(bg),
            None,
            &rhizo_app::vello::kurbo::Rect::new(x0, 0.0, x0 + 360.0, 400.0),
        );
        about::rake(&mut s, Vec2::new(x0 + 100.0, 60.0), 288.0, chrome::ink(bg).mix(bg, 0.5));
    }
    let Some(px) = render_scene_offscreen(&s, cw, ch, rhizo_core::Color::hex(0xffffff)) else { return };
    let at = |x: u32, y: u32| &px[((y * cw + x) * 4) as usize..][..3];
    assert!(at(180, 90)[0] > 200 && at(180, 90)[1] < 60, "red in chief");
    assert!(at(180, 300).iter().all(|c| *c > 240), "white in base");
    // Three white points rise into the red: under each peak it is white, between them red.
    for x in [100.0 + 240.0 / 6.0, 100.0 + 240.0 / 2.0, 100.0 + 240.0 * 5.0 / 6.0] {
        assert!(at(x as u32, 130).iter().all(|c| *c > 240), "a point at x={x}");
    }
    assert!(at(100 + 80, 130)[1] < 60, "red between the points");
    snapshot("rake-zoom.png", (cw, ch), &px);
}

/// A window of `size` (pixels at `dpr`): `built` fitted above the status
/// bar, `on_diagram` drawn over it in scene coordinates, then `on_window`
/// in window pixels. `None` without a GPU.
fn compose(
    built: &Built,
    (w, h): (u32, u32),
    dpr: f64,
    on_diagram: impl FnOnce(&mut Scene),
    on_window: impl FnOnce(&mut Scene, Vec2),
) -> Option<Vec<u8>> {
    let win = Vec2::new(w as f64, h as f64);
    let view = (win.x - 48.0 * dpr, win.y - (48.0 + chrome::BAR_HEIGHT) * dpr);
    let zoom = (view.0 / built.width).min(view.1 / built.height).min(dpr);
    let offset = Vec2::new(win.x - built.width * zoom, win.y - chrome::BAR_HEIGHT * dpr - built.height * zoom) / 2.0;
    let at = Affine::translate(offset) * Affine::scale(zoom);
    let mut scene = Scene::new();
    scene.append(&built.scene, Some(at));
    let mut over = Scene::new();
    on_diagram(&mut over);
    scene.append(&over, Some(at));
    on_window(&mut scene, win);
    render_scene_offscreen(&scene, w, h, built.background)
}

fn status(built: &Built, file: &str, selection: Option<String>, hints: Vec<String>) -> Status {
    Status {
        file: file.into(),
        nodes: built.nodes,
        edges: built.diagram.edges.len(),
        zoom: 1.0,
        selection,
        saved: 0.0,
        project: (0, 0),
        job: None,
        hints,
    }
}

/// The learnability pass: the tutorial and its card, the status bar's
/// hints, the palette's synonyms, kit types and nodes, the empty-canvas
/// hint and the gallery.
#[test]
fn learning_the_viewer_renders() {
    use rhizo_app::learn::{self, Seen};
    use rhizo_app::palette::{self, Entry, List, Selection};
    use rhizo_app::{gallery, ops};
    use rhizo_core::NodeId;
    use rhizo_lang::Editor;

    let ui = Fonts::system(rhizo_core::theme::FONT_STACK);
    let dpr = 2.0;
    let size = (2400u32, 1600u32);
    let win_canvas = |win: Vec2| rhizo_app::vello::kurbo::Rect::new(0.0, 0.0, win.x, win.y - chrome::BAR_HEIGHT * dpr);
    let mut glyphs = FontCache::default();
    let mut engine = Engine::default();

    // The tutorial at step 1, then with four steps done (step 5 next).
    let mut e = Editor::open(rhizo_lang::starters::starter("learn").unwrap());
    let mut shots = vec![("learn-step1.png", e.source().to_string())];
    let child = e.add_child(&NodeId::from("idea"), ops::NEW_LABEL).unwrap();
    let child = e.rename_new(&child, "Why").unwrap();
    let sib = e.add_after(&child, ops::NEW_LABEL).unwrap();
    e.rename_new(&sib, "How").unwrap();
    e.rename(&NodeId::from("idea"), "My plan").unwrap();
    e.connect(&NodeId::from("apples"), &NodeId::from("pears")).unwrap();
    shots.push(("learn-step5.png", e.source().to_string()));
    for (name, src) in shots {
        let built = engine.build("learn.rhizo", &src, Syntax::Kdl, Some("light")).unwrap();
        let done = learn::progress(&learn::start(), &built.document, Seen::default());
        let expect = if name.contains("step1") { "0 of 10" } else { "4 of 10" };
        assert_eq!(learn::count(&done), expect);
        let st = status(&built, "Learn rhizo.rhizo", None, palette::hints(Selection::Nothing, false));
        let Some(px) = compose(
            &built,
            size,
            dpr,
            |_| {},
            |s, win| {
                let card = learn::card(s, &ui, &mut glyphs, win_canvas(win), dpr, built.background, &done);
                assert!(card.x1 <= win.x && card.y0 >= 0.0, "the card sits in the corner");
                chrome::status_bar(s, &ui, &mut glyphs, win, dpr, built.background, &st);
            },
        ) else {
            return eprintln!("no GPU adapter, skipping");
        };
        snapshot(name, size, &px);
    }

    // Status-bar hints: one node selected, then a frame (an empty one shows its own hint).
    let built = example("product-strategy.rhizo", Some("light"));
    let sel = Some(chrome::describe_node("customers", "Customers"));
    let st = status(&built, "product-strategy.rhizo", sel, palette::hints(Selection::Node, false));
    let px = compose(
        &built,
        size,
        dpr,
        |_| {},
        |s, win| {
            chrome::status_bar(s, &ui, &mut glyphs, win, dpr, built.background, &st);
        },
    )
    .unwrap();
    snapshot("learn-hints-node.png", size, &px);
    let src = "rhizo version=2\ntitle \"Frames\"\nframe \"Production\" id=prod {\n}\nframe \"Team\" id=team {\n    node \"Ana\"\n    node \"Ben\"\n}\n";
    let built = engine.build("frames.rhizo", src, Syntax::Kdl, Some("light")).unwrap();
    assert_eq!(chrome::empty_frames(&built.diagram).len(), 1);
    let st = status(&built, "frames.rhizo", Some("Production (frame)".into()), palette::hints(Selection::Frame, false));
    let px = compose(
        &built,
        size,
        dpr,
        |s| chrome::empty_frame_hints(s, &built.diagram, &built.fonts, &mut FontCache::default()),
        |s, win| {
            chrome::status_bar(s, &ui, &mut glyphs, win, dpr, built.background, &st);
        },
    )
    .unwrap();
    snapshot("learn-hints-frame.png", size, &px);

    // The palette: a word finds commands; a kit type and a node by label.
    let infra = example("playground/07-typed-infra.rhizo", Some("light"));
    let kits = rhizo_kit::Kits::builtin();
    let mut extras: Vec<Entry> = kits
        .iter()
        .flat_map(|k| {
            k.types.values().map(|d| Entry {
                name: d.label.clone(),
                detail: format!("{} · {} kit", d.name, k.name),
                glyph: Some(rhizo_app::sidebar::Glyph::Kind(rhizo_app::project::Kind::of(&d.name))),
            })
        })
        .collect();
    extras.extend(infra.document.nodes.values().map(|n| Entry {
        name: n.label.clone(),
        detail: format!("node · {}", n.id),
        glyph: None,
    }));
    for (q, sel) in [("environment", Selection::Frame), ("bucket", Selection::Nothing)] {
        let rows = palette::rows(List::Commands, q, sel, &extras);
        assert!(!rows.is_empty(), "{q}");
        let px = compose(
            &infra,
            size,
            dpr,
            |_| {},
            |s, win| {
                palette::draw(s, &ui, &mut glyphs, win, dpr, infra.background, (List::Commands, q, 0), &rows);
            },
        )
        .unwrap();
        snapshot(&format!("learn-palette-{q}.png"), size, &px);
    }

    // The empty canvas, and the gallery, in both themes.
    for theme in ["light", "dark"] {
        let built = engine.build("Untitled.rhizo", "rhizo version=1\n", Syntax::Kdl, Some(theme)).unwrap();
        assert_eq!(built.nodes, 0);
        let st = status(&built, "Untitled.rhizo", None, palette::hints(Selection::Nothing, false));
        let px = compose(
            &built,
            size,
            dpr,
            |_| {},
            |s, win| {
                let links = chrome::empty_hint(
                    s,
                    &ui,
                    &mut glyphs,
                    win_canvas(win).size().to_vec2(),
                    dpr,
                    built.background,
                    None,
                );
                assert_eq!(links.len(), 2);
                chrome::status_bar(s, &ui, &mut glyphs, win, dpr, built.background, &st);
            },
        )
        .unwrap();
        snapshot(&format!("learn-empty-{theme}.png"), size, &px);

        let bg = rhizo_core::Theme::by_name(theme).unwrap().background;
        let thumbs = gallery::thumbnails(&mut Engine::default(), Some(theme));
        let mut s = Scene::new();
        let win = Vec2::new(size.0 as f64, size.1 as f64);
        let drawn = gallery::draw(&mut s, &ui, &mut glyphs, win, dpr, bg, &thumbs, Some(0), Some(3));
        assert_eq!(drawn.tiles.len(), thumbs.len());
        assert!(drawn.tiles.iter().all(|r| r.x0 >= 0.0 && r.x1 <= win.x && r.y1 <= win.y), "{theme}: tiles on screen");
        let px = render_scene_offscreen(&s, size.0, size.1, bg).unwrap();
        snapshot(&format!("learn-gallery-{theme}.png"), size, &px);

        // The welcome screen with its tutorial and samples cards.
        use rhizo_app::welcome::{self, Welcome};
        let recent = [Path::new("/Users/me/notes/product-strategy.rhizo").to_path_buf()];
        let w = Welcome { missing: None, recent: &recent };
        let mut s = Scene::new();
        let rects = welcome::draw(&mut s, &ui, &mut glyphs, win, dpr, bg, &w, None, Some(4));
        assert_eq!(rects.len(), w.items().len());
        let px = render_scene_offscreen(&s, size.0, size.1, bg).unwrap();
        snapshot(&format!("learn-welcome-{theme}.png"), size, &px);
    }
}
