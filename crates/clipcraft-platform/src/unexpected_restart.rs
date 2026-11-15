//! When did Windows last restart without shutting down cleanly?
//!
//! Windows logs "Kernel-Power" event 41 at the next start after a freeze, a power cut or a crash.
//! ClipCraft asks for the newest one, but only after a run that ended while the graphics card's
//! encoder was working, so most launches never run this. The query is Windows' own `wevtutil`
//! (no administrator rights needed to read the System log); nothing is installed or changed, and
//! only the time of the event is read.

use std::time::Duration;

/// How long to wait for `wevtutil` before giving up.
const QUERY_LIMIT: Duration = Duration::from_secs(8);

/// Unix seconds of the newest unexpected restart Windows has on record, or `None` when there is
/// none, the log cannot be read in time, or this is not Windows. Blocks until it has an answer
/// (at most a few seconds), so call it from a worker thread.
pub fn last_unexpected_restart() -> Option<u64> {
    imp::query()
}

/// The time of the first event in `wevtutil ... /f:xml` output, as Unix seconds.
///
/// The output contains `<TimeCreated SystemTime='2026-11-12T20:31:44.1234567Z'/>`. Anything else,
/// including a malformed or hostile string, gives `None`.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn event_time_unix(xml: &str) -> Option<u64> {
    let start = xml.find("SystemTime=")? + "SystemTime=".len();
    let rest = xml.get(start..)?;
    let quote = rest.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
    let body = rest.get(1..)?;
    let end = body.find(quote)?;
    iso_utc_to_unix(body.get(..end)?)
}

/// `2026-11-12T20:31:44.1234567Z` (the fraction is optional) to Unix seconds.
#[cfg_attr(not(any(windows, test)), allow(dead_code))]
fn iso_utc_to_unix(text: &str) -> Option<u64> {
    if text.len() > 40 || !text.is_ascii() {
        return None;
    }
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-');
    let year: i64 = date.next()?.parse().ok()?;
    let month: i64 = date.next()?.parse().ok()?;
    let day: i64 = date.next()?.parse().ok()?;
    if date.next().is_some() {
        return None;
    }
    let time = time.split('.').next()?;
    let mut time = time.split(':');
    let hour: i64 = time.next()?.parse().ok()?;
    let minute: i64 = time.next()?.parse().ok()?;
    let second: i64 = time.next()?.parse().ok()?;
    if time.next().is_some()
        || !(1970..=9999).contains(&year)
        || !(1..=12).contains(&month)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=60).contains(&second)
    {
        return None;
    }
    // Days since 1970-01-01 (the standard civil-calendar conversion).
    let shifted_year = if month <= 2 { year - 1 } else { year };
    let era = shifted_year.div_euclid(400);
    let year_of_era = shifted_year.rem_euclid(400);
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    let seconds = days
        .checked_mul(86_400)?
        .checked_add(hour * 3_600 + minute * 60 + second)?;
    u64::try_from(seconds).ok()
}

#[cfg(windows)]
mod imp {
    use super::{QUERY_LIMIT, event_time_unix};
    use std::os::windows::process::CommandExt;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;

    /// More than a single event needs; the rest is ignored.
    const MAX_OUTPUT_BYTES: usize = 64 * 1024;

    pub fn query() -> Option<u64> {
        // The system folder, not the search path, so a program of the same name cannot stand in.
        let system = std::env::var_os("SystemRoot").filter(|value| !value.is_empty())?;
        let program = PathBuf::from(system).join("System32").join("wevtutil.exe");
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("restart-check".into())
            .spawn(move || {
                let output = Command::new(program)
                    .args([
                        "qe",
                        "System",
                        "/q:*[System[Provider[@Name='Microsoft-Windows-Kernel-Power'] and (EventID=41)]]",
                        "/c:1",
                        "/rd:true",
                        "/f:xml",
                    ])
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .creation_flags(0x0800_0000)
                    .output();
                let _ = sender.send(output);
            })
            .ok()?;
        let output = receiver.recv_timeout(QUERY_LIMIT).ok()?.ok()?;
        if !output.status.success() {
            return None;
        }
        let bytes = output
            .stdout
            .get(..output.stdout.len().min(MAX_OUTPUT_BYTES))?;
        event_time_unix(&String::from_utf8_lossy(bytes))
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn query() -> Option<u64> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_real_event_line_gives_the_right_second() {
        // 2026-11-12T20:31:44Z, checked against the calendar: 20_769 days after 1970-01-01.
        let xml = "<Event><System><TimeCreated SystemTime='2026-11-12T20:31:44.1234567Z'/></System></Event>";
        assert_eq!(
            event_time_unix(xml),
            Some(20_769 * 86_400 + 20 * 3_600 + 31 * 60 + 44)
        );
        assert_eq!(iso_utc_to_unix("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(iso_utc_to_unix("2000-03-01T00:00:00Z"), Some(951_868_800));
        // A leap day, and double quotes.
        assert_eq!(iso_utc_to_unix("2024-02-29T12:00:00Z"), Some(1_709_208_000));
        assert_eq!(
            event_time_unix(r#"<TimeCreated SystemTime="2024-02-29T12:00:00.5Z"/>"#),
            Some(1_709_208_000)
        );
    }

    #[test]
    fn hostile_or_broken_text_gives_nothing_and_never_panics() {
        for text in [
            "",
            "SystemTime=",
            "SystemTime='",
            "SystemTime=''",
            "SystemTime='2026-11-12T20:31:44'",
            "SystemTime='2026-13-12T20:31:44Z'",
            "SystemTime='2026-11-32T20:31:44Z'",
            "SystemTime='2026-11-12T24:31:44Z'",
            "SystemTime='1969-12-31T23:59:59Z'",
            "SystemTime='10000-01-01T00:00:00Z'",
            "SystemTime='-2026-11-12T20:31:44Z'",
            "SystemTime='2026-11-12-01T20:31:44Z'",
            "SystemTime='2026-11-12T20:31:44:09Z'",
            "SystemTime='20２6-11-12T20:31:44Z'",
            "SystemTime=2026-11-12T20:31:44Z",
            "\u{0}\u{0}SystemTime='",
        ] {
            assert_eq!(event_time_unix(text), None, "{text:?}");
        }
        let long = format!("SystemTime='{}Z'", "9".repeat(100_000));
        assert_eq!(event_time_unix(&long), None);
        assert_eq!(iso_utc_to_unix(&"-".repeat(1_000)), None);
    }

    #[cfg(not(windows))]
    #[test]
    fn other_systems_report_no_restart() {
        assert_eq!(last_unexpected_restart(), None);
    }
}
