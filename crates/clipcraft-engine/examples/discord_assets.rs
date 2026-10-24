//! Maintainer-only CUT product asset exporter. Requires a new destination directory.
use clipcraft_engine::presence_art::{ASSET_KEYS, badge};
use std::{error::Error, fs, io::Write, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let path = PathBuf::from(args.next().ok_or("usage: discord_assets NEW_DIRECTORY")?);
    if args.next().is_some() {
        return Err("expected one new destination directory".into());
    }
    // create_dir refuses existing directories and symlinks; never overwrite user files.
    fs::create_dir(&path)?;
    for key in ASSET_KEYS {
        let file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path.join(format!("{key}.png")))?;
        let image = badge(key, 1024)?;
        let mut writer = std::io::BufWriter::new(file);
        image::DynamicImage::ImageRgba8(image).write_to(&mut writer, image::ImageFormat::Png)?;
        writer.flush()?;
    }
    let notice = "ClipCraft CUT product mark: original ClipCraft artwork, MIT. Developed from the maintainer-selected AI-assisted concept A. No publisher logos, game artwork or font glyphs are bundled. Supported game images are referenced on the Discord CDN and loaded by Discord. Upload clipcraft.png as the owned application icon and record its public CDN reference in PRODUCT_ICON_URL; a separate Rich Presence art-key upload is unnecessary. See LICENSE-MIT.\n";
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("NOTICE.txt"))?;
    file.write_all(notice.as_bytes())?;
    let mut license = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("LICENSE-MIT"))?;
    license.write_all(include_bytes!("../../../LICENSE-MIT"))?;
    Ok(())
}
