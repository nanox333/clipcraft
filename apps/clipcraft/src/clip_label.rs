//! The words a clip is shown with. ClipCraft names a new clip `<Game> <date> <time>`, which reads
//! badly twice over on a card that already shows the game: so the title is the game (or the name
//! someone gave the clip, without the file extension) and the date is written for people.

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
    second: u32,
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
            second,
        })
}

/// What to call the clip: the game for a clip ClipCraft named, otherwise the name it has, without
/// the extension.
pub(crate) fn title(file_name: &str, extension: &str) -> String {
    let stem = stem(file_name, extension);
    stamped(stem).map_or_else(|| stem.to_owned(), |stamp| stamp.name.to_owned())
}

/// Days since 1970-01-01 of a civil date (the inverse of `civil_from_days`).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let month_index = i64::from((month + 9) % 12);
    let day_of_year = (153 * month_index + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// When the clip was recorded, as a Unix time in UTC: from its name when ClipCraft made it (those
/// names are UTC), otherwise the time the file was last changed.
pub(crate) fn recorded_unix(
    file_name: &str,
    extension: &str,
    modified_unix_seconds: Option<u64>,
) -> Option<i64> {
    if let Some(stamp) = stamped(stem(file_name, extension)) {
        return Some(
            days_from_civil(i64::from(stamp.year), stamp.month, stamp.day) * 86_400
                + i64::from(stamp.hour) * 3_600
                + i64::from(stamp.minute) * 60
                + i64::from(stamp.second),
        );
    }
    modified_unix_seconds.and_then(|seconds| i64::try_from(seconds).ok())
}

/// A local calendar moment.
struct Local {
    days: i64,
    year: i64,
    month: u32,
    day: u32,
    hour: i64,
    minute: i64,
}

fn local(unix: i64, offset: i32) -> Local {
    let shifted = unix.saturating_add(i64::from(offset));
    let days = shifted.div_euclid(86_400);
    let seconds = shifted.rem_euclid(86_400);
    let (year, month, day) = crate::civil_from_days(days);
    Local {
        days,
        year,
        month,
        day,
        hour: seconds / 3_600,
        minute: seconds % 3_600 / 60,
    }
}

/// How the zone's offset and the clock are asked for (so tests can choose them).
pub(crate) struct Clock<'a> {
    pub now_unix: i64,
    pub offset: &'a dyn Fn(i64) -> i32,
}

impl Clock<'static> {
    /// The computer's own clock and time zone.
    pub(crate) fn system() -> Self {
        Self {
            now_unix: clipcraft_engine::unix_now() as i64,
            offset: &clipcraft_platform_offset,
        }
    }
}

fn clipcraft_platform_offset(unix: i64) -> i32 {
    clipcraft_platform::local_offset_seconds(unix)
}

/// `Today, 21:04`, `Yesterday, 18:22`, `15 Nov, 21:04`, or `31 Dec 2024, 23:59` for another year.
pub(crate) fn when_at(unix: Option<i64>, clock: &Clock<'_>) -> String {
    let Some(unix) = unix else {
        return "Unknown date".to_owned();
    };
    let at = local(unix, (clock.offset)(unix));
    let today = local(clock.now_unix, (clock.offset)(clock.now_unix));
    let time = format!("{:02}:{:02}", at.hour, at.minute);
    match today.days - at.days {
        0 => format!("Today, {time}"),
        1 => format!("Yesterday, {time}"),
        _ => {
            let month = MONTHS
                .get(usize::try_from(at.month.saturating_sub(1)).unwrap_or(0))
                .copied()
                .unwrap_or("");
            if at.year == today.year {
                format!("{} {month}, {time}", at.day)
            } else {
                format!("{} {month} {}, {time}", at.day, at.year)
            }
        }
    }
}

/// The long form for a clip's own page: `Sunday 15 November 2026 at 21:04`.
pub(crate) fn long_when_at(unix: Option<i64>, clock: &Clock<'_>) -> String {
    const WEEKDAYS: [&str; 7] = [
        "Sunday",
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
    ];
    const FULL_MONTHS: [&str; 12] = [
        "January",
        "February",
        "March",
        "April",
        "May",
        "June",
        "July",
        "August",
        "September",
        "October",
        "November",
        "December",
    ];
    let Some(unix) = unix else {
        return "Unknown date".to_owned();
    };
    let at = local(unix, (clock.offset)(unix));
    let weekday = WEEKDAYS
        .get(usize::try_from((at.days + 4).rem_euclid(7)).unwrap_or(0))
        .copied()
        .unwrap_or("");
    let month = FULL_MONTHS
        .get(usize::try_from(at.month.saturating_sub(1)).unwrap_or(0))
        .copied()
        .unwrap_or("");
    format!(
        "{weekday} {} {month} {} at {:02}:{:02}",
        at.day, at.year, at.hour, at.minute
    )
}

/// `when_at` for a clip, on the computer's clock.
pub(crate) fn when(file_name: &str, extension: &str, modified_unix_seconds: Option<u64>) -> String {
    when_at(
        recorded_unix(file_name, extension, modified_unix_seconds),
        &Clock::system(),
    )
}

/// `long_when_at` for a clip, on the computer's clock.
pub(crate) fn long_when(
    file_name: &str,
    extension: &str,
    modified_unix_seconds: Option<u64>,
) -> String {
    long_when_at(
        recorded_unix(file_name, extension, modified_unix_seconds),
        &Clock::system(),
    )
}

/// The heading a clip sits under when the library is in date order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum DateGroup {
    Today,
    Yesterday,
    EarlierThisWeek,
    EarlierThisMonth,
    Month(i64, u32),
    Unknown,
}

impl DateGroup {
    pub(crate) fn label(&self) -> String {
        const FULL_MONTHS: [&str; 12] = [
            "January",
            "February",
            "March",
            "April",
            "May",
            "June",
            "July",
            "August",
            "September",
            "October",
            "November",
            "December",
        ];
        match self {
            Self::Today => "Today".to_owned(),
            Self::Yesterday => "Yesterday".to_owned(),
            Self::EarlierThisWeek => "Earlier this week".to_owned(),
            Self::EarlierThisMonth => "Earlier this month".to_owned(),
            Self::Month(year, month) => format!(
                "{} {year}",
                FULL_MONTHS
                    .get(usize::try_from(month.saturating_sub(1)).unwrap_or(0))
                    .copied()
                    .unwrap_or("")
            ),
            Self::Unknown => "Unknown date".to_owned(),
        }
    }
}

/// Which heading a clip recorded at `unix` falls under.
pub(crate) fn group_at(unix: Option<i64>, clock: &Clock<'_>) -> DateGroup {
    let Some(unix) = unix else {
        return DateGroup::Unknown;
    };
    let at = local(unix, (clock.offset)(unix));
    let today = local(clock.now_unix, (clock.offset)(clock.now_unix));
    match today.days - at.days {
        i64::MIN..=0 => DateGroup::Today,
        1 => DateGroup::Yesterday,
        2..=6 => DateGroup::EarlierThisWeek,
        _ if at.year == today.year && at.month == today.month => DateGroup::EarlierThisMonth,
        _ => DateGroup::Month(at.year, at.month),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 2026-11-16 12:00 UTC.
    const NOW: i64 = 1_794_830_400;

    fn utc() -> Clock<'static> {
        Clock {
            now_unix: NOW,
            offset: &|_| 0,
        }
    }

    #[test]
    fn clips_named_by_clipcraft_are_shown_by_game() {
        assert_eq!(
            title("Counter-Strike 2 2026-11-15 21-04-12.mp4", "mp4"),
            "Counter-Strike 2"
        );
        assert_eq!(title("Desktop 2026-01-02 03-04-05.mkv", "mkv"), "Desktop");
    }

    #[test]
    fn names_are_read_as_utc_and_shown_in_the_local_zone() {
        let name = "Counter-Strike 2 2026-11-15 21-04-12.mp4";
        let unix = recorded_unix(name, "mp4", None).unwrap();
        assert_eq!(
            unix,
            days_from_civil(2026, 11, 15) * 86_400 + 21 * 3_600 + 4 * 60 + 12
        );
        assert_eq!(when_at(Some(unix), &utc()), "Yesterday, 21:04");
        // Two hours ahead of UTC: 23:04 the same evening.
        let ahead = Clock {
            now_unix: NOW,
            offset: &|_| 7_200,
        };
        assert_eq!(when_at(Some(unix), &ahead), "Yesterday, 23:04");
        // Three hours ahead pushes it over midnight into today.
        let more = Clock {
            now_unix: NOW,
            offset: &|_| 3 * 3_600,
        };
        assert_eq!(when_at(Some(unix), &more), "Today, 00:04");
        // Behind UTC: the clip belongs to the day before.
        let behind = Clock {
            now_unix: NOW,
            offset: &|_| -5 * 3_600,
        };
        assert_eq!(when_at(Some(unix), &behind), "Yesterday, 16:04");
        assert_eq!(
            long_when_at(Some(unix), &utc()),
            "Sunday 15 November 2026 at 21:04"
        );
    }

    #[test]
    fn friendly_dates_for_today_yesterday_this_year_and_other_years() {
        let day = 86_400;
        assert_eq!(when_at(Some(NOW - 3_600), &utc()), "Today, 11:00");
        assert_eq!(when_at(Some(NOW - day), &utc()), "Yesterday, 12:00");
        assert_eq!(when_at(Some(NOW - 20 * day), &utc()), "27 Oct, 12:00");
        assert_eq!(when_at(Some(NOW - 400 * day), &utc()), "12 Oct 2025, 12:00");
        assert_eq!(when_at(None, &utc()), "Unknown date");
        // A time in the future (a wrong clock) is simply today.
        assert_eq!(when_at(Some(NOW + 5 * day), &utc()), "20 Nov, 12:00");
    }

    #[test]
    fn groups_follow_local_days() {
        let day = 86_400;
        let group = |unix: i64| group_at(Some(unix), &utc());
        assert_eq!(group(NOW), DateGroup::Today);
        assert_eq!(group(NOW - 12 * 3_600), DateGroup::Today);
        assert_eq!(group(NOW - day), DateGroup::Yesterday);
        assert_eq!(group(NOW - 3 * day), DateGroup::EarlierThisWeek);
        // 2026-11-16 minus 10 days is still November.
        assert_eq!(group(NOW - 10 * day), DateGroup::EarlierThisMonth);
        assert_eq!(group(NOW - 20 * day), DateGroup::Month(2026, 10));
        assert_eq!(group_at(None, &utc()), DateGroup::Unknown);
        assert_eq!(DateGroup::Month(2026, 10).label(), "October 2026");
        assert_eq!(DateGroup::Today.label(), "Today");
    }

    #[test]
    fn other_names_are_kept_and_only_lose_their_extension() {
        assert_eq!(title("My best round.mp4", "mp4"), "My best round");
        assert_eq!(title("clip v1.2.MP4", "mp4"), "clip v1.2");
        assert_eq!(title("no extension", "mp4"), "no extension");
        assert_eq!(title(".mp4", "mp4"), ".mp4");
        assert_eq!(recorded_unix("My best round.mp4", "mp4", Some(5)), Some(5));
        assert_eq!(recorded_unix("My best round.mp4", "mp4", None), None);
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
            let _ = long_when(name, "mp4", Some(u64::MAX));
            let _ = when_at(Some(i64::MIN), &utc());
            let _ = group_at(Some(i64::MAX), &utc());
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
