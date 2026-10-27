use std::convert::Infallible;
use std::fmt;
use std::str::FromStr;

use dioxus::prelude::*;
use dioxus::router::{FromRouteSegments, ToRouteSegments};

use shared::api::tasks::Location;

use crate::api::encode_component;
use crate::pages::access_denied::AccessDenied;
use crate::pages::browse::Browse;
use crate::pages::home::Home;
use crate::pages::not_found::NotFound;
use crate::pages::signin_error::SigninError;
use crate::pages::usage::Usage;

#[derive(Routable, Clone, PartialEq, Debug)]
pub enum Route {
    #[route("/")]
    Home {},
    #[route("/browse/:volume/:..path")]
    Browse { volume: Segment, path: Segments },
    #[route("/usage/:id/:volume/:..path")]
    Usage {
        id: String,
        volume: Segment,
        path: Segments,
    },
    #[route("/access-denied")]
    AccessDenied {},
    #[route("/signin-error?:reason")]
    SigninError { reason: String },
    #[route("/:..segments")]
    NotFound { segments: Vec<String> },
}

/// The usage page of the scan `id` of `source`, at the scanned folder.
pub fn usage_route(id: &str, source: &Location) -> Route {
    Route::Usage {
        id: id.to_owned(),
        volume: source.volume.as_str().into(),
        path: Segments::of(&source.path),
    }
}

/// Navigates from anywhere, even from outside the router's components.
pub fn go(route: Route) {
    if let Some(router) = dioxus::router::root_router() {
        router.push(route);
    }
}

// Dioxus percent-encodes route segments but leaves `%` alone while decoding it on parse, so a
// name like `%41` would come back as `A`. These wrappers encode every segment like
// `encodeURIComponent`, which the router's own encoding then leaves untouched.

/// One name in a route. `Display` gives its URL form; use `as_str` for the name itself.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Segment(String);

impl Segment {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl From<String> for Segment {
    fn from(name: String) -> Self {
        Self(name)
    }
}

impl From<&str> for Segment {
    fn from(name: &str) -> Self {
        Self(name.to_owned())
    }
}

impl fmt::Display for Segment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&encode_component(&self.0))
    }
}

impl FromStr for Segment {
    type Err = Infallible;

    fn from_str(decoded: &str) -> Result<Self, Self::Err> {
        Ok(Self(decoded.to_owned()))
    }
}

/// A path inside a volume, one name per segment; empty for the volume root.
#[derive(Clone, PartialEq, Eq, Debug, Default)]
pub struct Segments(Vec<String>);

impl Segments {
    /// From the `/`-separated form the API uses.
    pub fn of(path: &str) -> Self {
        Self(
            path.split('/')
                .filter(|name| !name.is_empty())
                .map(str::to_owned)
                .collect(),
        )
    }

    pub fn names(&self) -> &[String] {
        &self.0
    }

    /// The `/`-separated form the API expects.
    pub fn joined(&self) -> String {
        self.0.join("/")
    }

    #[must_use]
    pub fn child(&self, name: &str) -> Self {
        let mut names = self.0.clone();
        names.push(name.to_owned());
        Self(names)
    }

    /// The first `len` names.
    #[must_use]
    pub fn prefix(&self, len: usize) -> Self {
        Self(self.0[..len.min(self.0.len())].to_vec())
    }
}

impl From<Vec<String>> for Segments {
    fn from(names: Vec<String>) -> Self {
        Self(names)
    }
}

impl ToRouteSegments for Segments {
    fn display_route_segments(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for name in &self.0 {
            write!(f, "/{}", encode_component(name))?;
        }
        Ok(())
    }
}

impl FromRouteSegments for Segments {
    type Err = Infallible;

    fn from_route_segments(decoded: &[&str]) -> Result<Self, Self::Err> {
        Ok(Self(decoded.iter().map(|&name| name.to_owned()).collect()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn browse(volume: &str, path: &[&str]) -> Route {
        Route::Browse {
            volume: volume.into(),
            path: path
                .iter()
                .map(|&name| name.to_owned())
                .collect::<Vec<_>>()
                .into(),
        }
    }

    #[test]
    fn browse_route_round_trips() {
        for route in [
            browse("media", &["é #?%", "x"]),
            browse("media", &["%41", "100%25", "a/b"]),
            browse("my files %20", &["😀 (1) & 'x'.txt"]),
            browse("media", &[]),
        ] {
            let url = route.to_string();
            assert_eq!(url.parse::<Route>().ok(), Some(route), "{url}");
        }
    }

    #[test]
    fn browse_route_formats_percent_encoded() {
        assert_eq!(
            browse("media", &["é #?%", "x"]).to_string(),
            "/browse/media/%C3%A9%20%23%3F%25/x"
        );
    }

    #[test]
    fn browse_route_without_path_is_the_volume_root() {
        assert_eq!(
            "/browse/media".parse::<Route>().ok(),
            Some(browse("media", &[]))
        );
        assert_eq!(
            "/browse/media/".parse::<Route>().ok(),
            Some(browse("media", &[]))
        );
    }

    #[test]
    fn segments_build_child_and_prefix_paths() {
        let path = Segments::from(vec!["a".to_owned(), "b c".to_owned()]);
        assert_eq!(path.joined(), "a/b c");
        assert_eq!(path.child("d").joined(), "a/b c/d");
        assert_eq!(path.prefix(1).joined(), "a");
        assert_eq!(path.prefix(5), path);
        assert_eq!(Segments::default().joined(), "");
        assert_eq!(Segments::of("a/b c"), path);
        assert_eq!(Segments::of(""), Segments::default());
    }

    fn usage(id: &str, volume: &str, path: &[&str]) -> Route {
        Route::Usage {
            id: id.to_owned(),
            volume: volume.into(),
            path: path
                .iter()
                .map(|&name| name.to_owned())
                .collect::<Vec<_>>()
                .into(),
        }
    }

    #[test]
    fn usage_route_round_trips_nested_paths() {
        let route = usage("0f3a", "my media", &["Films 2026", "é #?%", "x y"]);
        let url = route.to_string();
        assert_eq!(
            url,
            "/usage/0f3a/my%20media/Films%202026/%C3%A9%20%23%3F%25/x%20y"
        );
        assert_eq!(url.parse::<Route>().ok(), Some(route));
        assert_eq!(
            "/usage/0f3a/media".parse::<Route>().ok(),
            Some(usage("0f3a", "media", &[]))
        );
    }

    #[test]
    fn other_routes_parse() {
        assert_eq!("/".parse::<Route>().ok(), Some(Route::Home {}));
        assert_eq!(
            "/nope/x".parse::<Route>().ok(),
            Some(Route::NotFound {
                segments: vec!["nope".into(), "x".into()]
            })
        );
    }
}
