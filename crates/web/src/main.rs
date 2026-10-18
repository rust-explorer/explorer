mod api;
mod app;
mod components;
mod i18n;
mod pages;
mod routes;

fn main() {
    dioxus::launch(app::App);
}
