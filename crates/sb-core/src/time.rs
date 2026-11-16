use std::time::{SystemTime, UNIX_EPOCH};

const SECONDS_PER_DAY: u64 = 86_400;
const DAYS_FROM_YEAR_0_TO_1970: i64 = 719_468;
const DAYS_PER_400_YEARS: i64 = 146_097;

/// Formats `t` as `YYYYMMDDThhmmssZ` in UTC. Times before the epoch format as the epoch.
pub fn utc_compact(t: SystemTime) -> String {
    let seconds = t.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let (year, month, day) = civil_from_days((seconds / SECONDS_PER_DAY) as i64);
    let time_of_day = seconds % SECONDS_PER_DAY;
    let (hour, minute, second) = (
        time_of_day / 3600,
        time_of_day % 3600 / 60,
        time_of_day % 60,
    );
    format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
}

/// Proleptic Gregorian date for a day count since 1970-01-01 (Howard Hinnant's algorithm).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let shifted = days + DAYS_FROM_YEAR_0_TO_1970;
    let era = shifted / DAYS_PER_400_YEARS;
    let day_of_era = shifted - era * DAYS_PER_400_YEARS;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn should_format_utc_compact() {
        use std::time::{Duration, UNIX_EPOCH};
        assert_eq!(utc_compact(UNIX_EPOCH), "19700101T000000Z");
        assert_eq!(
            utc_compact(UNIX_EPOCH + Duration::from_secs(951_782_400)),
            "20000229T000000Z"
        );
        assert_eq!(
            utc_compact(UNIX_EPOCH + Duration::from_secs(1_763_216_553)),
            "20251115T142233Z"
        );
    }
}
