//! Puts the ClipCraft icon in the Windows executable (see `build_support/windows_icon.rs`).
include!("../../build_support/windows_icon.rs");

fn main() {
    let icon = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/brand/clipcraft.ico");
    if let Err(error) = embed_windows_icon(&icon) {
        println!("cargo:warning=the executable has no icon: {error}");
    }
}
