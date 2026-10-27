mod api;
mod app;
mod components;
mod conflict;
mod format;
mod i18n;
mod listing;
mod pages;
mod routes;
// Only the wasm build picks files, so most of the engine is unreachable on the host.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
mod uploads;

fn main() {
    dioxus::launch(app::App);
}
