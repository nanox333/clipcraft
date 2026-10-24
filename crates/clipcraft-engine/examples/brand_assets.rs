//! Reproducible CUT brand kit. Writes only into a newly created directory.
use clipcraft_engine::{SequenceText, TextRaster, brand};
use clipcraft_time::Tick;
use image::{Rgba, RgbaImage};
use std::{error::Error, fs, io::Write, path::Path};

fn write(path: &Path, bytes: &[u8]) -> Result<(), Box<dyn Error>> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(bytes)?;
    Ok(())
}

fn png(path: &Path, image: RgbaImage) -> Result<(), Box<dyn Error>> {
    let file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    let mut writer = std::io::BufWriter::new(file);
    image::DynamicImage::ImageRgba8(image).write_to(&mut writer, image::ImageFormat::Png)?;
    writer.flush()?;
    Ok(())
}

fn lockup(background: [u8; 3], foreground: [u8; 3]) -> Result<RgbaImage, Box<dyn Error>> {
    let mut image = RgbaImage::from_pixel(
        1120,
        320,
        Rgba([background[0], background[1], background[2], 255]),
    );
    image::imageops::overlay(&mut image, &brand::icon(320, brand::LIME)?, 0, 0);
    // White glyphs on black give a coverage mask. The video renderer's black shadow
    // contributes zero coverage, keeping this brand wordmark completely flat.
    let mut words = RgbaImage::from_pixel(800, 512, Rgba([0, 0, 0, 255]));
    let mut text = SequenceText::new(Tick::ZERO, Tick::from_units(1, 1), "ClipCraft");
    text.size = 250;
    text.color = [255, 255, 255];
    if let Some(raster) = TextRaster::render(&text, 800, 512)? {
        raster.draw(&mut words, &text, 1.0);
    }
    for pixel in words.pixels_mut() {
        let coverage = u32::from(pixel.0[0]);
        for channel in 0..3 {
            pixel.0[channel] = ((u32::from(foreground[channel]) * coverage
                + u32::from(background[channel]) * (255 - coverage)
                + 127)
                / 255) as u8;
        }
    }
    let words = image::imageops::crop_imm(&words, 0, 96, 800, 320).to_image();
    image::imageops::overlay(&mut image, &words, 320, 0);
    Ok(image)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let destination = args.next().ok_or("usage: brand_assets NEW_DIRECTORY")?;
    if args.next().is_some() {
        return Err("expected one new destination directory".into());
    }
    let directory = Path::new(&destination);
    fs::create_dir(directory)?;
    for (name, ink) in [
        ("lime", brand::LIME),
        ("black", brand::CHARCOAL),
        ("white", brand::PAPER),
    ] {
        write(
            &directory.join(format!("mark-{name}.svg")),
            brand::svg(ink).as_bytes(),
        )?;
        png(
            &directory.join(format!("mark-{name}-1024.png")),
            brand::icon(1024, ink)?,
        )?;
    }
    for size in [16, 24, 32, 48, 64, 128, 256] {
        png(
            &directory.join(format!("icon-{size}.png")),
            brand::icon(size, brand::LIME)?,
        )?;
    }
    png(
        &directory.join("discord-clipcraft.png"),
        clipcraft_engine::presence_art::badge("clipcraft", 1024)?,
    )?;
    let light = lockup(brand::PAPER, brand::CHARCOAL)?;
    let dark = lockup(brand::CHARCOAL, brand::PAPER)?;
    png(&directory.join("lockup-light.png"), light.clone())?;
    png(&directory.join("lockup-dark.png"), dark.clone())?;
    let mut board = RgbaImage::from_pixel(1200, 960, Rgba([226, 227, 223, 255]));
    image::imageops::overlay(&mut board, &light, 40, 40);
    image::imageops::overlay(&mut board, &dark, 40, 400);
    for (index, size) in [16, 24, 32, 48, 64, 128].into_iter().enumerate() {
        image::imageops::overlay(
            &mut board,
            &brand::icon(size, brand::CHARCOAL)?,
            80 + index as i64 * 170,
            790,
        );
    }
    png(&directory.join("brand-board.png"), board)?;
    write(&directory.join("NOTICE.txt"), b"ClipCraft CUT mark: original ClipCraft artwork, developed from the user-selected AI-assisted concept A; MIT. Inter SemiBold: SIL OFL 1.1, see OFL-Inter.md. No publisher or Medal artwork.\n")?;
    write(
        &directory.join("OFL-Inter.md"),
        include_bytes!("../../../assets/fonts/OFL-Inter.md"),
    )?;
    Ok(())
}
