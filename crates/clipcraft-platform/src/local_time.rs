//! The computer's own time zone: how far local time is from UTC at a given moment (summer time
//! included). ClipCraft names clips with UTC times; this is what turns "21:04" into the hour the
//! person actually lived it, and decides what counts as "today".
//!
//! The one `unsafe` call on each system is documented where it is made.
#![cfg_attr(any(windows, unix), allow(unsafe_code))]

/// Seconds to add to a Unix time (seconds since 1970, UTC) to get the local wall-clock time at that
/// moment. Zero when the system cannot say.
pub fn local_offset_seconds(unix_seconds: i64) -> i32 {
    imp::offset(unix_seconds)
}

#[cfg(unix)]
mod imp {
    pub fn offset(unix_seconds: i64) -> i32 {
        // `time_t` is 64 bits almost everywhere; where it is narrower, an out-of-range time is
        // answered with zero instead of wrapping.
        let time = unix_seconds as libc::time_t;
        // A no-op on 64-bit `time_t`, and the check that matters on a narrower one.
        #[allow(clippy::useless_conversion)]
        let widened = i64::from(time);
        if widened != unix_seconds {
            return 0;
        }
        let mut parts = std::mem::MaybeUninit::<libc::tm>::zeroed();
        // SAFETY: `localtime_r` reads `time` and writes a `tm` into the buffer given, which is valid
        // and large enough for the call; it keeps no pointer to either.
        let done = unsafe { libc::localtime_r(&time, parts.as_mut_ptr()) };
        if done.is_null() {
            return 0;
        }
        // SAFETY: the call succeeded, so it filled the whole structure.
        let parts = unsafe { parts.assume_init() };
        i32::try_from(parts.tm_gmtoff).unwrap_or(0)
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::Foundation::{FILETIME, SYSTEMTIME};
    use windows::Win32::System::Time::{
        FileTimeToSystemTime, SystemTimeToFileTime, SystemTimeToTzSpecificLocalTime,
    };

    /// 100-nanosecond steps between 1601-01-01 (Windows) and 1970-01-01 (Unix).
    const EPOCH_GAP: i64 = 116_444_736_000_000_000;

    fn ticks(time: FILETIME) -> i64 {
        (i64::from(time.dwHighDateTime) << 32) | i64::from(time.dwLowDateTime)
    }

    pub fn offset(unix_seconds: i64) -> i32 {
        let Some(utc_ticks) = unix_seconds
            .checked_mul(10_000_000)
            .and_then(|value| value.checked_add(EPOCH_GAP))
            .filter(|value| *value > 0)
        else {
            return 0;
        };
        let file_time = FILETIME {
            dwLowDateTime: (utc_ticks & 0xFFFF_FFFF) as u32,
            dwHighDateTime: (utc_ticks >> 32) as u32,
        };
        let mut utc = SYSTEMTIME::default();
        let mut local = SYSTEMTIME::default();
        let mut local_file = FILETIME::default();
        // SAFETY: each call reads or writes only the values passed to it, which are valid for the
        // call and owned by this function.
        let converted = unsafe {
            FileTimeToSystemTime(&file_time, &mut utc).is_ok()
                && SystemTimeToTzSpecificLocalTime(None, &utc, &mut local).is_ok()
                && SystemTimeToFileTime(&local, &mut local_file).is_ok()
        };
        if !converted {
            return 0;
        }
        i32::try_from((ticks(local_file) - ticks(file_time)) / 10_000_000).unwrap_or(0)
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub fn offset(_unix_seconds: i64) -> i32 {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_is_a_real_zone_offset_and_never_panics_on_odd_times() {
        for time in [
            0_i64,
            1_794_830_400,
            1_700_000_000,
            -86_400,
            i64::MAX,
            i64::MIN,
        ] {
            let offset = local_offset_seconds(time);
            assert!(
                (-15 * 3_600..=15 * 3_600).contains(&offset),
                "{time}: {offset}"
            );
        }
    }
}
