/// The image formats the server makes thumbnails for, by extension.
pub const THUMBNAIL_EXTENSIONS: [&str; 5] = ["jpg", "jpeg", "png", "gif", "webp"];

/// Whether the server makes a thumbnail for `name`, by its extension.
#[must_use]
pub fn has_thumbnail(name: &str) -> bool {
    let Some((stem, extension)) = name.rsplit_once('.') else {
        return false;
    };
    !stem.is_empty()
        && THUMBNAIL_EXTENSIONS
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thumbnail_names() {
        for (name, expected) in [
            ("a.jpg", true),
            ("a.JPEG", true),
            ("b.Png", true),
            ("c.gif", true),
            ("d.webp", true),
            ("photo.tar.jpg", true),
            ("e.heic", false),
            ("f.svg", false),
            ("c.avif", false),
            ("d.tiff", false),
            ("g.txt", false),
            ("jpg", false),
            (".jpg", false),
            ("", false),
        ] {
            assert_eq!(has_thumbnail(name), expected, "{name}");
        }
    }
}
