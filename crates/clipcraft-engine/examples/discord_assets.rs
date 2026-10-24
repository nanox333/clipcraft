//! Maintainer-only original asset exporter. Requires a new destination directory.
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
    let notice = "ClipCraft CUT product mark and legacy typographic game placeholders: original ClipCraft artwork, MIT. CUT was developed from the maintainer-selected AI-assisted concept A.\nInter SemiBold glyphs: SIL Open Font License 1.1; see OFL-Inter.md.\nNo game logos or Medal assets. Upload the PNGs to your owned Discord application's Rich Presence Art Assets using the filename stem as the key.\n";
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("NOTICE.txt"))?;
    file.write_all(notice.as_bytes())?;
    let mut font_license = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path.join("OFL-Inter.md"))?;
    font_license.write_all(include_bytes!("../../../assets/fonts/OFL-Inter.md"))?;
    Ok(())
}
