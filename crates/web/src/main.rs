mod api;
mod app;
mod components;
mod format;
mod i18n;
mod listing;
mod pages;
mod routes;
#[allow(dead_code, unused_imports)]
mod uploads;

fn main() {
    dioxus::launch(app::App);
}
