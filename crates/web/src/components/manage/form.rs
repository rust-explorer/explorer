use dioxus_i18n::t;

use crate::api::ApiFailure;
use crate::components::common::error_message;
use crate::names::NameError;

pub(super) const BUTTON: &str = "rounded-md border border-slate-300 px-3 py-1.5 text-sm font-medium text-slate-700 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-700 dark:text-slate-200 dark:hover:bg-slate-800";
pub(super) const INPUT: &str = "mt-1 block w-full rounded-md border border-slate-300 bg-white px-3 py-1.5 text-sm text-slate-900 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 dark:border-slate-700 dark:bg-slate-950 dark:text-white";
pub(super) const PRIMARY: &str = "rounded-md bg-indigo-600 px-3 py-1.5 text-sm font-semibold text-white hover:bg-indigo-500 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-indigo-600 disabled:cursor-not-allowed disabled:bg-slate-300 dark:disabled:bg-slate-700";

/// Why a name dialog stays open: a name refused before sending, or the server's answer.
pub(super) enum Problem {
    Name(NameError),
    Server(ApiFailure),
}

impl Problem {
    pub(super) fn message(&self) -> String {
        match self {
            Self::Name(NameError::Empty) => t!("name-error-empty"),
            Self::Name(NameError::Slash) => t!("name-error-slash"),
            Self::Name(NameError::Dot) => t!("name-error-dot"),
            Self::Name(NameError::Reserved) => t!("name-error-reserved"),
            Self::Name(NameError::TooLong) => t!("name-error-too-long"),
            Self::Server(failure) => error_message(failure),
        }
    }
}
