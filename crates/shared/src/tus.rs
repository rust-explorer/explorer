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

/// Bytes read from each end of a file for its [`fingerprint`].
pub const FINGERPRINT_SAMPLE: u64 = 64 * 1024;

/// Tells apart two files with the same name, size and date before an upload is resumed:
/// a 64-bit FNV-1a hash of the first and last [`FINGERPRINT_SAMPLE`] bytes (`head`, `tail`),
/// as 16 hex digits. Not a cryptographic hash; it only guards against mistakes.
#[must_use]
pub fn fingerprint(head: &[u8], tail: &[u8]) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in head.iter().chain(tail) {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// Whether `value` has the shape [`fingerprint`] gives.
#[must_use]
pub fn is_fingerprint(value: &str) -> bool {
    value.len() == 16
        && value
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints() {
        assert_eq!(fingerprint(b"", b""), "cbf29ce484222325");
        assert_eq!(fingerprint(b"a", b""), "af63dc4c8601ec8c");
        assert_ne!(fingerprint(b"ab", b"c"), fingerprint(b"ab", b"d"));
        assert!(is_fingerprint(&fingerprint(b"x", b"y")));
        for bad in [
            "",
            "CBF29CE484222325",
            "cbf29ce48422232",
            "cbf29ce4842223250",
        ] {
            assert!(!is_fingerprint(bad), "{bad}");
        }
    }

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
