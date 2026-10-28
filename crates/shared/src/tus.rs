use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

pub const TUS_VERSION: &str = "1.0.0";

/// Builds an `Upload-Metadata` header value.
#[must_use]
pub fn encode_metadata(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(key, value)| format!("{key} {}", STANDARD.encode(value)))
        .collect::<Vec<_>>()
        .join(",")
}

/// Parses an `Upload-Metadata` header value; a key without a value maps to an empty string.
#[must_use]
pub fn decode_metadata(header: &str) -> Option<Vec<(String, String)>> {
    header
        .split(',')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once(' ').unwrap_or((pair, ""));
            let value = String::from_utf8(STANDARD.decode(value).ok()?).ok()?;
            Some((key.to_owned(), value))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metadata_round_trips() {
        let pairs = [
            ("volume", "media"),
            ("path", "Album 2026/Été, été.jpg"),
            ("conflict", "keep_both"),
            ("empty", ""),
        ];
        let header = encode_metadata(&pairs);
        assert!(header.starts_with("volume bWVkaWE=,path "), "{header}");
        let decoded = decode_metadata(&header).unwrap();
        let decoded: Vec<(&str, &str)> = decoded
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        assert_eq!(decoded, pairs);
        assert_eq!(decode_metadata("").unwrap(), []);
    }

    #[test]
    fn metadata_rejects_bad_base64() {
        assert_eq!(decode_metadata("path !!!"), None);
        assert_eq!(decode_metadata("path YS50eHQ"), None);
        assert_eq!(decode_metadata("path /w=="), None);
        assert_eq!(decode_metadata("path YS50eHQ= extra"), None);
    }

    #[test]
    fn metadata_key_without_value_is_empty() {
        assert_eq!(
            decode_metadata("flag,path YS50eHQ=").unwrap(),
            [
                ("flag".to_owned(), String::new()),
                ("path".to_owned(), "a.txt".to_owned())
            ]
        );
    }
}
