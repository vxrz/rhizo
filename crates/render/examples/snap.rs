//! cargo run -p rhizo-render --example snap -- input.rhizo out.png [theme]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let src = std::fs::read_to_string(&args[1]).unwrap();
    let doc = rhizo_lang::load(&src, rhizo_lang::Syntax::Kdl).into_result().unwrap().0;
    let theme = rhizo_render::theme_for(&doc, args.get(3).map(String::as_str));
    let fonts = rhizo_render::Fonts::system(theme.font_family);
    let d = rhizo_render::layout(&doc, &theme, &fonts);
    let svg = rhizo_render::to_svg(&d, &Default::default());
    std::fs::write(args[2].replace(".png", ".svg"), &svg).unwrap();
    std::fs::write(&args[2], rhizo_render::svg_to_png(&svg, &fonts, 1.5).unwrap()).unwrap();
}
