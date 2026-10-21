use std::fmt::Write as _;

/// Same character set as `encodeURIComponent`.
pub fn encode_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            let _ = write!(encoded, "%{byte:02X}");
        }
    }
    encoded
}

pub fn entries_url(volume: &str, path: &str) -> String {
    format!(
        "/api/volumes/{}/entries?path={}",
        encode_component(volume),
        encode_component(path)
    )
}

pub fn download_url(volume: &str, path: &str) -> String {
    files_url("download", volume, path)
}

pub fn zip_url(volume: &str, path: &str) -> String {
    files_url("zip", volume, path)
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn upload_url(id: &str) -> String {
    format!("/files/uploads/{}", encode_component(id))
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn folders_url(volume: &str) -> String {
    format!("/api/volumes/{}/folders", encode_component(volume))
}

fn files_url(action: &str, volume: &str, path: &str) -> String {
    format!(
        "/files/{action}?volume={}&path={}",
        encode_component(volume),
        encode_component(path)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls_percent_encode() {
        assert_eq!(
            entries_url("media", "é #?%/x"),
            "/api/volumes/media/entries?path=%C3%A9%20%23%3F%25%2Fx"
        );
        assert_eq!(
            download_url("media", "é #?%/x"),
            "/files/download?volume=media&path=%C3%A9%20%23%3F%25%2Fx"
        );
        assert_eq!(
            zip_url("media", "é #?%/x"),
            "/files/zip?volume=media&path=%C3%A9%20%23%3F%25%2Fx"
        );
        assert_eq!(
            entries_url("my files&?", ""),
            "/api/volumes/my%20files%26%3F/entries?path="
        );
        assert_eq!(
            download_url("a&b", "c&d=1"),
            "/files/download?volume=a%26b&path=c%26d%3D1"
        );
    }
}
