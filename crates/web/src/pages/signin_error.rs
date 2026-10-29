use dioxus::prelude::*;
use dioxus_i18n::t;

use crate::components::{LanguagePicker, Notice, Shell};

/// Maps the server's reason code to a message id; unknown codes never reach the UI verbatim.
pub fn message_key(reason: &str) -> &'static str {
    match reason {
        "provider_error" => "signin-error-provider-error",
        "state_mismatch" => "signin-error-state-mismatch",
        "exchange_failed" => "signin-error-exchange-failed",
        "invalid_token" => "signin-error-invalid-token",
        "missing_groups_claim" => "signin-error-missing-groups-claim",
        // Sent by the server's rate limit, whose message the API error shares.
        "too_many_requests" => "error-too_many_requests",
        _ => "signin-error-unknown",
    }
}

#[component]
pub fn SigninError(reason: String) -> Element {
    rsx! {
        Shell {
            div { class: "flex w-full max-w-md flex-col gap-4",
                Notice {
                    title: t!("signin-error-title"),
                    message: t!(message_key(&reason)),
                    a {
                        href: "/auth/login",
                        class: "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500",
                        {t!("try-again")}
                    }
                }
                div { class: "max-w-48",
                    LanguagePicker { id: "signin-error-language" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_reason_has_its_own_message() {
        for reason in [
            "provider_error",
            "state_mismatch",
            "exchange_failed",
            "invalid_token",
            "missing_groups_claim",
        ] {
            assert_eq!(
                message_key(reason),
                format!("signin-error-{}", reason.replace('_', "-"))
            );
        }
    }

    #[test]
    fn rate_limited_sign_in_shows_the_api_error_message() {
        assert_eq!(message_key("too_many_requests"), "error-too_many_requests");
    }

    #[test]
    fn unknown_reasons_get_a_generic_message() {
        assert_eq!(message_key(""), "signin-error-unknown");
        assert_eq!(message_key("<script>"), "signin-error-unknown");
    }
}
