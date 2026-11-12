//! The application icon files: `clipcraft.ico` for the Windows executable (PNG pictures at the
//! sizes Windows asks for) and the PNGs a Linux desktop entry uses. Writes only into a newly
//! created directory: `cargo run -p clipcraft-engine --example app_icon NEW_DIRECTORY`.
use clipcraft_engine::brand;
use image::RgbaImage;
use std::{error::Error, fs, io::Cursor, path::Path};

const ICO_SIZES: [u32; 7] = [16, 24, 32, 48, 64, 128, 256];
const PNG_SIZES: [u32; 3] = [48, 128, 256];

fn png_bytes(image: RgbaImage) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut bytes = Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(image).write_to(&mut bytes, image::ImageFormat::Png)?;
    Ok(bytes.into_inner())
}

/// An .ico whose pictures are all PNG, which every Windows since Vista reads at any size.
fn ico() -> Result<Vec<u8>, Box<dyn Error>> {
    let pictures = ICO_SIZES
        .iter()
        .map(|size| png_bytes(brand::icon(*size, brand::LIME)?))
        .collect::<Result<Vec<_>, _>>()?;
    let mut out = Vec::new();
    out.extend_from_slice(&0_u16.to_le_bytes());
    out.extend_from_slice(&1_u16.to_le_bytes());
    out.extend_from_slice(&(pictures.len() as u16).to_le_bytes());
    let mut offset = 6 + 16 * pictures.len() as u32;
    for (size, picture) in ICO_SIZES.iter().zip(&pictures) {
        // 256 is written as 0, as the format says.
        let edge = if *size >= 256 { 0 } else { *size as u8 };
        out.extend_from_slice(&[edge, edge, 0, 0]);
        out.extend_from_slice(&1_u16.to_le_bytes());
        out.extend_from_slice(&32_u16.to_le_bytes());
        out.extend_from_slice(&(picture.len() as u32).to_le_bytes());
        out.extend_from_slice(&offset.to_le_bytes());
        offset += picture.len() as u32;
    }
    for picture in pictures {
        out.extend_from_slice(&picture);
    }
    Ok(out)
}

fn main() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args_os().skip(1);
    let destination = args.next().ok_or("usage: app_icon NEW_DIRECTORY")?;
    if args.next().is_some() {
        return Err("expected one new destination directory".into());
    }
    let directory = Path::new(&destination);
    fs::create_dir(directory)?;
    fs::write(directory.join("clipcraft.ico"), ico()?)?;
    for size in PNG_SIZES {
        fs::write(
            directory.join(format!("clipcraft-{size}.png")),
            png_bytes(brand::icon(size, brand::LIME)?)?,
        )?;
    }
    Ok(())
}
