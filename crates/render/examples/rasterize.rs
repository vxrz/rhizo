//! Rasterize any SVG with rhizo's renderer (used for brand assets).
//! cargo run -p rhizo-render --example rasterize -- in.svg out.png <width>
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let svg = std::fs::read_to_string(&args[1]).expect("read svg");
    let width: f32 = args.get(3).and_then(|w| w.parse().ok()).unwrap_or(1024.0);
    let fonts = rhizo_render::Fonts::system(rhizo_core::theme::FONT_STACK);
    let tree = usvg::Tree::from_str(&svg, &usvg::Options { fontdb: fonts.database(), ..Default::default() }).unwrap();
    let scale = width / tree.size().width();
    std::fs::write(&args[2], rhizo_render::svg_to_png(&svg, &fonts, scale).unwrap()).unwrap();
}
