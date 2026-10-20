//! Capturing a Wayland screen through the ScreenCast portal and PipeWire.
//!
//! These are ignored by default: the one portal backend tried here (xdg-desktop-portal-wlr 0.7.1 on
//! headless sway) refuses to start a share. Run them with `--ignored` on a desktop that works.
//! They need a running Wayland compositor with a PipeWire server and a portal backend (sway
//! headless with xdg-desktop-portal-wlr, as set up in docs/wayland.md) and `swaymsg` to change
//! what is on screen. They skip, saying so, when that is not available.
#![cfg(target_os = "linux")]

use std::process::Command;
use std::time::{Duration, Instant};

use clipcraft_capture::CaptureSource;
use clipcraft_platform::CaptureOptions;

fn sway_paint(color: &str) -> bool {
    Command::new("swaymsg")
        .args(["output", "*", "bg", color, "solid_color"])
        .output()
        .is_ok_and(|output| output.status.success())
}

fn centre(frame: &clipcraft_capture::CaptureFrame) -> [u8; 4] {
    let at = (frame.height as usize / 2) * frame.stride + (frame.width as usize / 2) * 4;
    let mut pixel = [0; 4];
    if let Some(bytes) = frame.bgra.get(at..at + 4) {
        pixel.copy_from_slice(bytes);
    }
    pixel
}

fn wait_for(source: &mut dyn CaptureSource, bgra: [u8; 3]) -> bool {
    let end = Instant::now() + Duration::from_secs(8);
    while Instant::now() < end {
        let Ok(frame) = source.capture() else {
            return false;
        };
        if frame.validate().is_ok() && centre(&frame)[..3] == bgra {
            return true;
        }
    }
    false
}

#[test]
#[ignore = "needs a desktop whose portal backend works; run with --ignored (see docs/wayland.md)"]
fn the_screen_is_captured_through_the_portal_and_follows_what_is_drawn() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() || !sway_paint("#102030") {
        eprintln!("skipped: no sway session to capture");
        return;
    }
    let targets = clipcraft_platform::list_targets().expect("targets");
    if !targets.iter().any(|target| target.id == "wayland:portal") {
        eprintln!("skipped: no screen-cast portal answers on this desktop");
        return;
    }
    let mut source = clipcraft_platform::open("wayland:portal", &CaptureOptions::default())
        .unwrap_or_else(|error| panic!("the share starts: {error:?}"));
    let first = source.capture().expect("a picture");
    assert!(first.validate().is_ok());
    assert!(first.width >= 640 && first.height >= 360);
    assert_eq!(source.target().width, first.width);

    // sway paints #RRGGBB; the picture is BGRA.
    assert!(
        wait_for(source.as_mut(), [0x30, 0x20, 0x10]),
        "first colour"
    );
    assert!(sway_paint("#c0ffee"));
    assert!(
        wait_for(source.as_mut(), [0xee, 0xff, 0xc0]),
        "second colour"
    );
}

#[test]
#[ignore = "needs a desktop whose portal backend works; run with --ignored (see docs/wayland.md)"]
fn a_still_screen_keeps_returning_pictures_and_stops_cleanly() {
    if std::env::var_os("WAYLAND_DISPLAY").is_none() || !sway_paint("#445566") {
        eprintln!("skipped: no sway session to capture");
        return;
    }
    let targets = clipcraft_platform::list_targets().expect("targets");
    if !targets.iter().any(|target| target.id == "wayland:portal") {
        eprintln!("skipped: no screen-cast portal answers on this desktop");
        return;
    }
    let mut source = clipcraft_platform::open("wayland:portal", &CaptureOptions::default())
        .unwrap_or_else(|error| panic!("the share starts: {error:?}"));
    let started = Instant::now();
    let mut last = None;
    for _ in 0..30 {
        let frame = source.capture().expect("a picture");
        assert!(last.is_none_or(|last| frame.captured_at >= last));
        last = Some(frame.captured_at);
    }
    assert!(started.elapsed() < Duration::from_secs(10));
    let before = Instant::now();
    drop(source);
    assert!(
        before.elapsed() < Duration::from_secs(5),
        "stopping is quick"
    );
}
