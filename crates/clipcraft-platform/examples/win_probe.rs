//! Temporary: finds which Windows call crashes on a machine without a screen.
#[cfg(windows)]
fn say(text: &str) {
    use std::io::Write;
    println!("{text}");
    let _ = std::io::stdout().flush();
}

#[cfg(windows)]
fn main() {
    use windows_capture::monitor::Monitor;
    use windows_capture::window::Window;
    say("enumerate monitors");
    let monitors = Monitor::enumerate().unwrap_or_default();
    say(&format!("{} monitors", monitors.len()));
    for monitor in &monitors {
        say("device_name");
        say(&format!("{:?}", monitor.device_name()));
        say("width");
        say(&format!("{:?}", monitor.width()));
        say("height");
        say(&format!("{:?}", monitor.height()));
        say("name");
        say(&format!("{:?}", monitor.name()));
    }
    say("enumerate windows");
    let windows = Window::enumerate().unwrap_or_default();
    say(&format!("{} windows", windows.len()));
    for window in &windows {
        say("title");
        say(&format!("{:?}", window.title()));
        say("size");
        say(&format!("{:?} {:?}", window.width(), window.height()));
    }
    say("foreground");
    match Window::foreground() {
        Ok(window) => {
            say("foreground title");
            say(&format!("{:?}", window.title()));
            say("foreground is_valid");
            say(&format!("{}", window.is_valid()));
        }
        Err(error) => say(&format!("no foreground: {error:?}")),
    }
    say("platform list_targets");
    say(&format!(
        "{:?}",
        clipcraft_platform::list_targets().map(|t| t.len())
    ));
    say("platform active_window");
    say(&format!(
        "{:?}",
        clipcraft_platform::active_window().map(|w| w.is_some())
    ));
    say("done");
}

#[cfg(not(windows))]
fn main() {}
