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

pub fn search_url(volume: &str, query: &str) -> String {
    format!(
        "/api/volumes/{}/search?q={}",
        encode_component(volume),
        encode_component(query)
    )
}

pub fn download_url(volume: &str, path: &str) -> String {
    files_url("download", volume, path)
}

pub fn zip_url(volume: &str, path: &str) -> String {
    files_url("zip", volume, path)
}

pub fn zip_selection_url(volume: &str, dir: &str, names: &[&str]) -> String {
    let mut url = files_url("zip", volume, dir);
    for name in names {
        url.push_str("&name=");
        url.push_str(&encode_component(name));
    }
    url
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn upload_url(id: &str) -> String {
    format!("/files/uploads/{}", encode_component(id))
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn folders_url(volume: &str) -> String {
    format!("/api/volumes/{}/folders", encode_component(volume))
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn rename_url(volume: &str) -> String {
    format!("/api/volumes/{}/rename", encode_component(volume))
}

pub const FAVORITES_URL: &str = "/api/favorites";

pub fn favorite_url(volume: &str, path: &str) -> String {
    format!(
        "{FAVORITES_URL}?volume={}&path={}",
        encode_component(volume),
        encode_component(path)
    )
}

pub const TASKS_URL: &str = "/api/tasks";

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub fn task_url(id: &str) -> String {
    format!("{TASKS_URL}/{}", encode_component(id))
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
        assert_eq!(
            zip_selection_url("media", "Films/é", &["a b", "c&d"]),
            "/files/zip?volume=media&path=Films%2F%C3%A9&name=a%20b&name=c%26d"
        );
        assert_eq!(
            search_url("my media", "a b&c é"),
            "/api/volumes/my%20media/search?q=a%20b%26c%20%C3%A9"
        );
        assert_eq!(rename_url("my files"), "/api/volumes/my%20files/rename");
        assert_eq!(task_url("0a/b"), "/api/tasks/0a%2Fb");
        assert_eq!(
            favorite_url("my files", "é/x"),
            "/api/favorites?volume=my%20files&path=%C3%A9%2Fx"
        );
    }
}
