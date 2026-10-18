use dioxus::prelude::*;

use crate::pages::access_denied::AccessDenied;
use crate::pages::home::Home;
use crate::pages::signin_error::SigninError;

#[derive(Routable, Clone, PartialEq, Debug)]
pub enum Route {
    #[route("/access-denied")]
    AccessDenied {},
    #[route("/signin-error?:reason")]
    SigninError { reason: String },
    #[route("/:..segments")]
    Home { segments: Vec<String> },
}
