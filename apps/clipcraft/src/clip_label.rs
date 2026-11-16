//! The words a clip is shown with. ClipCraft names a new clip `<Game> <date> <time>`, which reads
//! badly twice over on a card that already shows the game: so the title is the game (or the name
//! someone gave the clip, without the file extension) and the date is written for people.

use crate::format_date;

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// The file name without its extension (when it ends with `.extension`).
fn stem<'a>(file_name: &'a str, extension: &str) -> &'a str {
    let cut = file_name.len().saturating_sub(extension.len() + 1);
    match (file_name.get(cut..), file_name.get(..cut)) {
        (Some(tail), Some(head))
            if tail.starts_with('.')
                && tail[1..].eq_ignore_ascii_case(extension)
                && !head.is_empty() =>
        {
            head
        }
        _ => file_name,
    }
}

/// A recording time read from a name of the form `<name> YYYY-MM-DD HH-MM-SS`.
struct Stamped<'a> {
    name: &'a str,
    year: u32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
}

fn stamped(stem: &str) -> Option<Stamped<'_>> {
    // " 2026-11-15 21-04-12" is 20 characters.
    let cut = stem.len().checked_sub(20)?;
    let name = stem.get(..cut)?;
    let tail = stem.get(cut..)?;
    let bytes = tail.as_bytes();
    if name.trim().is_empty() || bytes.len() != 20 {
        return None;
    }
    let shape = b" nnnn-nn-nn nn-nn-nn";
    for (byte, wanted) in bytes.iter().zip(shape) {
        let fits = if *wanted == b'n' {
            byte.is_ascii_digit()
        } else {
            byte == wanted
        };
        if !fits {
            return None;
        }
    }
    let number = |from: usize, to: usize| tail.get(from..to)?.parse::<u32>().ok();
    let (year, month, day) = (number(1, 5)?, number(6, 8)?, number(9, 11)?);
    let (hour, minute, second) = (number(12, 14)?, number(15, 17)?, number(18, 20)?);
    ((1970..=9999).contains(&year)
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && hour < 24
        && minute < 60
        && second < 61)
        .then_some(Stamped {
            name: name.trim(),
            year,
            month,
            day,
            hour,
            minute,
        })
}

/// What to call the clip: the game for a clip ClipCraft named, otherwise the name it has, without
/// the extension.
pub(crate) fn title(file_name: &str, extension: &str) -> String {
    let stem = stem(file_name, extension);
    stamped(stem).map_or_else(|| stem.to_owned(), |stamp| stamp.name.to_owned())
}

/// When the clip was recorded, for people (`15 Nov, 21:04`): from its name when ClipCraft made it,
/// otherwise the date the file was last changed. The year is added when it is not the year of the
/// file's last change.
pub(crate) fn when(file_name: &str, extension: &str, modified_unix_seconds: Option<u64>) -> String {
    let Some(stamp) = stamped(stem(file_name, extension)) else {
        return format_date(modified_unix_seconds);
    };
    let month = MONTHS
        .get(usize::try_from(stamp.month - 1).unwrap_or(0))
        .copied()
        .unwrap_or("");
    let modified_year = modified_unix_seconds
        .and_then(|seconds| i64::try_from(seconds / 86_400).ok())
        .map(|days| crate::civil_from_days(days).0);
    let year = if modified_year.is_some_and(|year| year != i64::from(stamp.year)) {
        format!(" {}", stamp.year)
    } else {
        String::new()
    };
    format!(
        "{} {month}{year}, {:02}:{:02}",
        stamp.day, stamp.hour, stamp.minute
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clips_named_by_clipcraft_are_shown_by_game_and_a_friendly_time() {
        let name = "Counter-Strike 2 2026-11-15 21-04-12.mp4";
        assert_eq!(title(name, "mp4"), "Counter-Strike 2");
        // 2026-11-16 12:00 UTC: the same year, so no year is added.
        assert_eq!(when(name, "mp4", Some(1_794_830_400)), "15 Nov, 21:04");
        assert_eq!(title("Desktop 2026-01-02 03-04-05.mkv", "mkv"), "Desktop");
        // A different year from the file's last change says which year it was.
        assert_eq!(
            when(
                "Valorant 2024-12-31 23-59-59.mp4",
                "mp4",
                Some(1_794_830_400)
            ),
            "31 Dec 2024, 23:59"
        );
    }

    #[test]
    fn other_names_are_kept_and_only_lose_their_extension() {
        assert_eq!(title("My best round.mp4", "mp4"), "My best round");
        assert_eq!(title("clip v1.2.MP4", "mp4"), "clip v1.2");
        assert_eq!(title("no extension", "mp4"), "no extension");
        assert_eq!(title(".mp4", "mp4"), ".mp4");
        assert_eq!(when("My best round.mp4", "mp4", Some(0)), "1970-01-01");
        assert_eq!(when("My best round.mp4", "mp4", None), "Unknown date");
    }

    #[test]
    fn near_misses_and_hostile_names_are_left_alone_and_never_panic() {
        for name in [
            "2026-11-15 21-04-12.mp4",
            "Game 2026-13-15 21-04-12.mp4",
            "Game 2026-11-15 25-04-12.mp4",
            "Game 2026-11-15 21-04.mp4",
            "Game 2026-11-15  21-04-12.mp4",
            "Game 2026-11-15 21-04-1\u{e9}.mp4",
            "\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}\u{e9}.mp4",
            "",
            " ",
        ] {
            let _ = title(name, "mp4");
            let _ = when(name, "mp4", Some(u64::MAX));
        }
        assert_eq!(
            title("2026-11-15 21-04-12.mp4", "mp4"),
            "2026-11-15 21-04-12"
        );
        assert_eq!(
            title("Game 2026-13-15 21-04-12.mp4", "mp4"),
            "Game 2026-13-15 21-04-12"
        );
    }
}
