//! Dev tool: render `assets/icon.svg` to a macOS `.iconset` of PNGs.
//!
//! Run from the repo root:
//!     cargo run --manifest-path tools/icon-gen/Cargo.toml -- assets/icon.svg assets/xpress.iconset
//! then:
//!     iconutil -c icns assets/xpress.iconset -o assets/AppIcon.icns

use resvg::{tiny_skia, usvg};
use std::path::Path;

const SIZES: &[(u32, &str)] = &[
    (16, "icon_16x16"),
    (32, "icon_16x16@2x"),
    (32, "icon_32x32"),
    (64, "icon_32x32@2x"),
    (128, "icon_128x128"),
    (256, "icon_128x128@2x"),
    (256, "icon_256x256"),
    (512, "icon_256x256@2x"),
    (512, "icon_512x512"),
    (1024, "icon_512x512@2x"),
];

fn main() {
    let mut args = std::env::args().skip(1);
    let svg_path = args.next().unwrap_or_else(|| "assets/icon.svg".into());
    let out_dir = args.next().unwrap_or_else(|| "assets/xpress.iconset".into());

    let data = std::fs::read(&svg_path).expect("read svg");
    let tree = usvg::Tree::from_data(&data, &usvg::Options::default()).expect("parse svg");
    let base = tree.size().width();

    std::fs::create_dir_all(&out_dir).expect("create iconset dir");

    for (px, name) in SIZES {
        let mut pixmap = tiny_skia::Pixmap::new(*px, *px).expect("alloc pixmap");
        let scale = *px as f32 / base;
        let transform = tiny_skia::Transform::from_scale(scale, scale);
        resvg::render(&tree, transform, &mut pixmap.as_mut());
        let out = Path::new(&out_dir).join(format!("{name}.png"));
        pixmap.save_png(&out).expect("save png");
        println!("  {} ({}x{})", out.display(), px, px);
    }
    println!("Done. Build the icns with:\n  iconutil -c icns {out_dir} -o assets/AppIcon.icns");
}
