//! Original moving test window for the offline workflow check, on a private Xvfb display only.
#[cfg(all(unix, not(target_os = "macos")))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::time::{Duration, Instant};
    use x11rb::{connection::Connection, protocol::xproto::*, wrapper::ConnectionExt as _};
    if std::env::args().skip(1).collect::<Vec<_>>() != ["--private-display"] {
        return Err("run only on a private Xvfb display with --private-display".into());
    }
    let (conn, screen) = x11rb::connect(None)?;
    let root = conn
        .setup()
        .roots
        .get(screen)
        .ok_or("display has no screen")?;
    let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    };
    let list = atom(b"_NET_CLIENT_LIST")?;
    let manager = atom(b"_NET_SUPPORTING_WM_CHECK")?;
    if !conn
        .get_property(false, root.root, manager, AtomEnum::ANY, 0, 1)?
        .reply()?
        .value
        .is_empty()
    {
        return Err("refusing a managed desktop; use a private Xvfb display".into());
    }
    let existing = conn
        .get_property(false, root.root, list, AtomEnum::WINDOW, 0, 512)?
        .reply()?;
    if existing.value32().is_some_and(|mut ids| {
        ids.any(|id| {
            conn.get_window_attributes(id)
                .ok()
                .is_some_and(|c| c.reply().is_ok())
        })
    }) {
        return Err(
            "refusing to replace an existing client list; use a fresh private display".into(),
        );
    }
    let window = conn.generate_id()?;
    let gc = conn.generate_id()?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        window,
        root.root,
        1480,
        120,
        640,
        360,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new().background_pixel(0x102030),
    )?
    .check()?;
    conn.change_property32(
        PropMode::REPLACE,
        window,
        atom(b"_NET_WM_PID")?,
        AtomEnum::CARDINAL,
        &[std::process::id()],
    )?
    .check()?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"ClipCraft workflow game",
    )?
    .check()?;
    conn.change_property8(
        PropMode::REPLACE,
        window,
        atom(b"_NET_WM_NAME")?,
        atom(b"UTF8_STRING")?,
        b"ClipCraft workflow game",
    )?
    .check()?;
    conn.change_property32(
        PropMode::REPLACE,
        root.root,
        list,
        AtomEnum::WINDOW,
        &[window],
    )?
    .check()?;
    conn.create_gc(gc, window, &CreateGCAux::new())?.check()?;
    conn.map_window(window)?.check()?;
    conn.flush()?;
    println!("Generated game window ready");
    let start = Instant::now();
    let mut frame = 0;
    while start.elapsed() < Duration::from_secs(180) {
        conn.change_gc(gc, &ChangeGCAux::new().foreground(0x102030))?;
        conn.poly_fill_rectangle(
            window,
            gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: 640,
                height: 360,
            }],
        )?;
        conn.change_gc(gc, &ChangeGCAux::new().foreground(0xb6ff00))?;
        conn.poly_fill_rectangle(
            window,
            gc,
            &[Rectangle {
                x: ((frame * 4) % 550) as i16,
                y: 100,
                width: 90,
                height: 140,
            }],
        )?;
        conn.flush()?;
        frame += 1;
        std::thread::sleep(Duration::from_millis(16));
    }
    conn.delete_property(root.root, list)?.check()?;
    conn.destroy_window(window)?.check()?;
    conn.flush()?;
    Ok(())
}

#[cfg(not(all(unix, not(target_os = "macos"))))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    Err("the generated workflow window requires private X11/Xvfb; it is not a game or capture backend".into())
}
