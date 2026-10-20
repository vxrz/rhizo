//! Renders the examples on the GPU without a window and checks the pixels.
//! Skips (passes) on machines without a GPU adapter.

use std::path::Path;

use rhizo_app::{Engine, render_offscreen};

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
    let differs = px.chunks_exact(4).filter(|p| p[..3] != [bg.r, bg.g, bg.b]).count();
    // Nodes, edges and text cover a real share of the canvas.
    assert!(differs * 20 > (w * h) as usize, "{name}: only {differs} drawn pixels of {}", w * h);

    // Every label is drawn inside its node: some pixels there have the text color.
    for n in built.diagram.nodes.iter().filter(|n| !n.label.is_empty()) {
        let r = n.rect;
        let close = |p: &[u8]| p.iter().zip([n.text.r, n.text.g, n.text.b]).all(|(a, b)| a.abs_diff(b) < 48);
        let (x0, y0, x1, y1) = (r.x as u32, r.y as u32, r.right() as u32, r.bottom() as u32);
        let hits = (y0..y1.min(h))
            .flat_map(|y| (x0..x1.min(w)).map(move |x| (x, y)))
            .filter(|&(x, y)| close(at(x, y)))
            .count();
        assert!(hits > 3, "{name}: no text pixels inside `{}`", n.id);
    }

    if let Ok(dir) = std::env::var("RHIZO_SNAPSHOT_DIR") {
        let out = Path::new(&dir).join(name).with_extension("rgba");
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
