//! Renders an SVG to a square PNG: `cargo run -p rc-kit --example render_svg -- in.svg out.png 512`.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    let (input, output, size) = (&args[1], &args[2], args[3].parse::<u32>().unwrap());
    let data = std::fs::read(input).unwrap();
    let tree = resvg::usvg::Tree::from_data(&data, &resvg::usvg::Options::default()).unwrap();
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).unwrap();
    let scale = size as f32 / tree.size().width();
    resvg::render(&tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.save_png(output).unwrap();
}
