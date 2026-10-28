use unic_langid::LanguageIdentifier;

const UNITS_EN: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
const UNITS_FR: [&str; 5] = ["o", "ko", "Mo", "Go", "To"];

/// SI size: one decimal below 10, none above, with the locale's unit labels and separator.
pub fn format_size(bytes: u64, locale: &LanguageIdentifier) -> String {
    let units = match locale.language.as_str() {
        "fr" => UNITS_FR,
        _ => UNITS_EN,
    };
    let separator = match locale.language.as_str() {
        "fr" | "es" => ',',
        _ => '.',
    };
    let bytes = u128::from(bytes);
    let mut divisor = 1u128;
    for (index, unit) in units.iter().enumerate() {
        let tenths = (bytes * 10 + divisor / 2) / divisor;
        if tenths < 100 {
            return match tenths % 10 {
                0 => format!("{} {unit}", tenths / 10),
                decimal => format!("{}{separator}{decimal} {unit}", tenths / 10),
            };
        }
        let whole = (bytes + divisor / 2) / divisor;
        if whole < 1000 || index == units.len() - 1 {
            return format!("{whole} {unit}");
        }
        divisor *= 1000;
    }
    unreachable!("the last unit always returns")
}

pub fn format_speed(bytes_per_sec: u64, locale: &LanguageIdentifier) -> String {
    format!("{}/s", format_size(bytes_per_sec, locale))
}

#[cfg(target_arch = "wasm32")]
pub fn now_ms() -> i64 {
    #[expect(
        clippy::cast_possible_truncation,
        reason = "Date.now() is a whole number of milliseconds"
    )]
    let now = js_sys::Date::now() as i64;
    now
}

#[cfg(not(target_arch = "wasm32"))]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| {
            i64::try_from(elapsed.as_millis()).unwrap_or(i64::MAX)
        })
}

/// Today's time, "yesterday", the day and month this year, otherwise the full date.
/// The wasm build reads "yesterday" from the active translations, so call it while rendering.
/// Host builds have no time zone or `Intl`, so they give the UTC ISO date.
#[cfg(target_arch = "wasm32")]
pub fn format_modified(ms: i64, now_ms: i64, locale: &LanguageIdentifier) -> String {
    use js_sys::Intl::{DateTimeFormat, DateTimeFormatOptions, DayFormat, MonthFormat};
    use js_sys::Intl::{NumericFormat, YearFormat};
    use js_sys::{Array, Date};

    #[expect(
        clippy::cast_precision_loss,
        reason = "millisecond timestamps stay far below 2^53"
    )]
    let date_at = |ms: i64| Date::new(&(ms as f64).into());
    let day_of = |date: &Date| (date.get_full_year(), date.get_month(), date.get_date());
    let date = date_at(ms);
    let today = date_at(now_ms);
    let yesterday = date_at(now_ms);
    yesterday.set_date(yesterday.get_date() - 1);

    let options = DateTimeFormatOptions::new();
    if day_of(&date) == day_of(&today) {
        options.set_hour(NumericFormat::Numeric);
        options.set_minute(NumericFormat::TwoDigit);
    } else if day_of(&date) == day_of(&yesterday) {
        return dioxus_i18n::t!("modified-yesterday");
    } else {
        options.set_day(DayFormat::Numeric);
        options.set_month(MonthFormat::Short);
        if date.get_full_year() != today.get_full_year() {
            options.set_year(YearFormat::Numeric);
        }
    }
    let locales = Array::of1(&locale.to_string().into());
    DateTimeFormat::new(&locales, &options)
        .format()
        .call1(&js_sys::wasm_bindgen::JsValue::UNDEFINED, &date)
        .ok()
        .and_then(|formatted| formatted.as_string())
        .unwrap_or_default()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn format_modified(ms: i64, _now_ms: i64, _locale: &LanguageIdentifier) -> String {
    let (year, month, day) = civil_from_days(ms.div_euclid(86_400_000));
    format!("{year:04}-{month:02}-{day:02}")
}

/// Gregorian date of a day count since 1970-01-01 (Howard Hinnant's algorithm).
#[cfg(not(target_arch = "wasm32"))]
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use unic_langid::langid;

    use super::*;

    #[test]
    fn format_size_by_locale() {
        let en = langid!("en-US");
        assert_eq!(format_size(0, &en), "0 B");
        assert_eq!(format_size(999, &en), "999 B");
        assert_eq!(format_size(1000, &en), "1 kB");
        assert_eq!(format_size(82_000, &en), "82 kB");
        assert_eq!(format_size(9_800_000_000, &en), "9.8 GB");
        assert_eq!(format_size(1_200_000_000_000, &en), "1.2 TB");

        let fr = langid!("fr-FR");
        assert_eq!(format_size(9_800_000_000, &fr), "9,8 Go");
        assert_eq!(format_size(532_000_000, &fr), "532 Mo");

        let es = langid!("es-ES");
        assert_eq!(format_size(1_200_000, &es), "1,2 MB");
    }

    #[test]
    fn format_size_rounds_into_the_next_unit() {
        let en = langid!("en-US");
        assert_eq!(format_size(9_960_000, &en), "10 MB");
        assert_eq!(format_size(999_500, &en), "1 MB");
        assert_eq!(format_size(2_500_000_000_000_000, &en), "2500 TB");
        assert_eq!(format_size(u64::MAX, &en), "18446744 TB");
    }

    #[test]
    fn format_speed_by_locale() {
        assert_eq!(format_speed(48_000_000, &langid!("en-US")), "48 MB/s");
        assert_eq!(format_speed(48_000_000, &langid!("fr-FR")), "48 Mo/s");
    }

    #[test]
    fn format_modified_host_fallback() {
        let en = langid!("en-US");
        let now = 1_760_000_000_000;
        assert_eq!(format_modified(0, now, &en), "1970-01-01");
        assert_eq!(format_modified(1_728_734_400_000, now, &en), "2024-10-12");
        assert_eq!(format_modified(1_709_164_800_000, now, &en), "2024-02-29");
        assert_eq!(format_modified(1_709_251_199_999, now, &en), "2024-02-29");
        assert_eq!(format_modified(-1, now, &en), "1969-12-31");
        assert_eq!(format_modified(now, now, &langid!("fr-FR")), "2025-10-09");
    }
}
