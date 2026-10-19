mod api;
mod app;
mod components;
mod format;
mod i18n;
mod listing;
mod pages;
mod routes;

fn main() {
    dioxus::launch(app::App);
}
