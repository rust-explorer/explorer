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
}
