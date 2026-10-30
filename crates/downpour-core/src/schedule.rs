//! Start/stop window arithmetic for the scheduler.
//!
//! The window is expressed in local minutes and a weekday bitmask
//! (bit 0 = Monday .. bit 6 = Sunday), matching the settings UI. Overnight
//! windows (`22:00`–`06:00`) are supported.

/// Parse `HH:MM` into minutes since midnight.
#[must_use]
pub fn parse_minutes(value: &str) -> Option<u16> {
    let (hours, minutes) = value.trim().split_once(':')?;
    let hours: u16 = hours.trim().parse().ok()?;
    let minutes: u16 = minutes.trim().parse().ok()?;
    if hours > 23 || minutes > 59 {
        return None;
    }
    Some(hours * 60 + minutes)
}

/// Whether `weekday`/`minutes` fall inside the window.
///
/// `days_mask` bits: 0 = Monday, 1 = Tuesday, ... 6 = Sunday.
#[must_use]
pub fn window_allows(
    days_mask: u8,
    start_minutes: u16,
    end_minutes: u16,
    weekday: u8,
    minutes: u16,
) -> bool {
    let allowed_on = |day: u8| day < 7 && (days_mask & (1 << day)) != 0;
    let weekday = weekday % 7;

    if start_minutes <= end_minutes {
        allowed_on(weekday) && minutes >= start_minutes && minutes < end_minutes
    } else if minutes >= start_minutes {
        // The window started today and crosses midnight.
        allowed_on(weekday)
    } else if minutes < end_minutes {
        // This is the tail of a window that started yesterday.
        allowed_on((weekday + 6) % 7)
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALL_DAYS: u8 = 0b0111_1111;
    const MONDAY: u8 = 0;

    #[test]
    fn parses_times() {
        assert_eq!(parse_minutes("00:00"), Some(0));
        assert_eq!(parse_minutes("09:30"), Some(570));
        assert_eq!(parse_minutes("23:59"), Some(1439));
        assert_eq!(parse_minutes("24:00"), None);
        assert_eq!(parse_minutes("9:60"), None);
        assert_eq!(parse_minutes("nope"), None);
    }

    #[test]
    fn daytime_window() {
        assert!(window_allows(ALL_DAYS, 540, 1020, MONDAY, 600));
        assert!(!window_allows(ALL_DAYS, 540, 1020, MONDAY, 500));
        assert!(!window_allows(ALL_DAYS, 540, 1020, MONDAY, 1020));
    }

    #[test]
    fn overnight_window_uses_the_start_day() {
        // Monday 22:00 - 06:00.
        assert!(window_allows(ALL_DAYS, 1320, 360, MONDAY, 1380));
        assert!(window_allows(ALL_DAYS, 1320, 360, MONDAY, 120)); // Monday 02:00 is before the window
        // Tuesday 02:00 is inside Monday's overnight window.
        assert!(window_allows(ALL_DAYS, 1320, 360, 1, 120));
        // Tuesday 12:00 is outside.
        assert!(!window_allows(ALL_DAYS, 1320, 360, 1, 720));
    }

    #[test]
    fn weekday_mask_is_respected() {
        let mondays_only = 0b0000_0001;
        assert!(window_allows(mondays_only, 540, 1020, 0, 600));
        assert!(!window_allows(mondays_only, 540, 1020, 1, 600));
        // Tuesday 02:00 belongs to Monday's overnight window.
        assert!(window_allows(mondays_only, 1320, 360, 1, 120));
    }

    #[test]
    fn empty_mask_never_allows() {
        assert!(!window_allows(0, 0, 1439, 0, 600));
    }
}
