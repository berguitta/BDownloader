//! Development tasks.
//!
//! `cargo run -p xtask -- icons` renders `assets/logo.svg` into the PNG and
//! ICO files used by the app (window, executable, tray). The outputs are
//! committed, so normal builds do not need this.

use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};

fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("icons") => icons(),
        _ => bail!("usage: cargo run -p xtask -- icons"),
    }
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("workspace root").to_path_buf()
}

fn render(tree: &resvg::usvg::Tree, size: u32) -> Result<resvg::tiny_skia::Pixmap> {
    let mut pixmap = resvg::tiny_skia::Pixmap::new(size, size).context("pixmap")?;
    let scale = size as f32 / tree.size().width();
    resvg::render(tree, resvg::tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    Ok(pixmap)
}

fn icons() -> Result<()> {
    let assets = root().join("assets");
    let svg = std::fs::read(assets.join("logo.svg"))?;
    let tree = resvg::usvg::Tree::from_data(&svg, &resvg::usvg::Options::default())?;

    for (size, name) in [(256, "logo-256.png"), (64, "logo-64.png"), (32, "tray-32.png")] {
        render(&tree, size)?.save_png(assets.join(name))?;
        println!("wrote assets/{name}");
    }

    let mut dir = ico::IconDir::new(ico::ResourceType::Icon);
    for size in [16, 20, 24, 32, 40, 48, 64, 128, 256] {
        let pixmap = render(&tree, size)?;
        // tiny-skia stores premultiplied RGBA; ICO wants straight alpha.
        let rgba: Vec<u8> = pixmap
            .pixels()
            .iter()
            .flat_map(|p| {
                let c = p.demultiply();
                [c.red(), c.green(), c.blue(), c.alpha()]
            })
            .collect();
        let image = ico::IconImage::from_rgba_data(size, size, rgba);
        dir.add_entry(ico::IconDirEntry::encode(&image)?);
    }
    dir.write(std::fs::File::create(assets.join("icon.ico"))?)?;
    println!("wrote assets/icon.ico");
    Ok(())
}
