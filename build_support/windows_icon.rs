// Shared by the build scripts of the programs that carry the ClipCraft icon on Windows.
//
// Turns `assets/brand/clipcraft.ico` into a compiled resource (`.res`) that the MSVC linker
// accepts as an input, so the .exe itself has the icon (Explorer, shortcuts, the pinned taskbar
// button). Plain Rust: no resource compiler or other tool is needed. Included with `include!`.

use std::path::Path;

/// The pictures of an .ico: width, height, colour planes, bits per pixel and the bytes.
struct Picture {
    width: u8,
    height: u8,
    planes: u16,
    bits: u16,
    data: Vec<u8>,
}

fn read_u16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(bytes.get(at..at + 2)?.try_into().ok()?))
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?))
}

fn pictures(ico: &[u8]) -> Result<Vec<Picture>, String> {
    let bad = || "clipcraft.ico is not a valid icon file".to_owned();
    if read_u16(ico, 0) != Some(0) || read_u16(ico, 2) != Some(1) {
        return Err(bad());
    }
    let count = usize::from(read_u16(ico, 4).ok_or_else(bad)?);
    if count == 0 || count > 16 {
        return Err(bad());
    }
    let mut out = Vec::new();
    for index in 0..count {
        let entry = 6 + index * 16;
        let size = read_u32(ico, entry + 8).ok_or_else(bad)? as usize;
        let offset = read_u32(ico, entry + 12).ok_or_else(bad)? as usize;
        out.push(Picture {
            width: *ico.get(entry).ok_or_else(bad)?,
            height: *ico.get(entry + 1).ok_or_else(bad)?,
            planes: read_u16(ico, entry + 4).ok_or_else(bad)?,
            bits: read_u16(ico, entry + 6).ok_or_else(bad)?,
            data: ico
                .get(offset..offset.checked_add(size).ok_or_else(bad)?)
                .ok_or_else(bad)?
                .to_vec(),
        });
    }
    Ok(out)
}

/// One resource with a numeric type and name, in the `.res` layout (language-neutral).
fn entry(out: &mut Vec<u8>, kind: u16, name: u16, data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out.extend_from_slice(&32_u32.to_le_bytes()); // header size
    out.extend_from_slice(&0xFFFF_u16.to_le_bytes());
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&0xFFFF_u16.to_le_bytes());
    out.extend_from_slice(&name.to_le_bytes());
    out.extend_from_slice(&0_u32.to_le_bytes()); // data version
    out.extend_from_slice(&0x1030_u16.to_le_bytes()); // MOVEABLE | PURE | DISCARDABLE
    out.extend_from_slice(&0_u16.to_le_bytes()); // language: neutral
    out.extend_from_slice(&0_u32.to_le_bytes()); // version
    out.extend_from_slice(&0_u32.to_le_bytes()); // characteristics
    out.extend_from_slice(data);
    out.resize(out.len().div_ceil(4) * 4, 0);
}

/// The compiled resource for the icon: every picture as an RT_ICON, and one RT_GROUP_ICON named 1
/// (the first icon group, which Windows uses as the program's icon).
fn icon_resource(ico: &[u8]) -> Result<Vec<u8>, String> {
    let pictures = pictures(ico)?;
    let mut out = Vec::new();
    // Every .res starts with an empty entry.
    entry(&mut out, 0, 0, &[]);
    out.truncate(32);
    let mut group = Vec::new();
    group.extend_from_slice(&0_u16.to_le_bytes());
    group.extend_from_slice(&1_u16.to_le_bytes());
    group.extend_from_slice(&(pictures.len() as u16).to_le_bytes());
    for (index, picture) in pictures.iter().enumerate() {
        let id = index as u16 + 1;
        entry(&mut out, 3, id, &picture.data);
        group.extend_from_slice(&[picture.width, picture.height, 0, 0]);
        group.extend_from_slice(&picture.planes.to_le_bytes());
        group.extend_from_slice(&picture.bits.to_le_bytes());
        group.extend_from_slice(&(picture.data.len() as u32).to_le_bytes());
        group.extend_from_slice(&id.to_le_bytes());
    }
    entry(&mut out, 14, 1, &group);
    Ok(out)
}

/// Links the icon into the executables of the package being built, when the target is Windows
/// with the MSVC linker. Other targets have nothing to do.
fn embed_windows_icon(icon: &Path) -> Result<(), String> {
    println!("cargo:rerun-if-changed={}", icon.display());
    println!("cargo:rerun-if-changed=build.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").is_ok_and(|os| os == "windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    if !windows || !msvc {
        return Ok(());
    }
    let ico = std::fs::read(icon).map_err(|error| format!("cannot read {}: {error}", icon.display()))?;
    let resource = icon_resource(&ico)?;
    let out_dir = std::env::var_os("OUT_DIR").ok_or("OUT_DIR is not set")?;
    let path = Path::new(&out_dir).join("clipcraft-icon.res");
    std::fs::write(&path, resource).map_err(|error| format!("cannot write the icon resource: {error}"))?;
    println!("cargo:rustc-link-arg-bins={}", path.display());
    Ok(())
}
