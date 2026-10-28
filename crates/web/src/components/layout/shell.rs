use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::auth::Me;

use super::user_menu::UserMenu;
use crate::components::common::icons::MenuIcon;
use crate::routes::Route;

const SIDEBAR_ID: &str = "sidebar";

/// Page frame shared by every route; the user menu appears once the session is known.
/// With a `sidebar`, the page becomes sidebar plus content, and the sidebar is a drawer below `md`.
/// `search` and `actions` sit in the header, before the user menu.
#[component]
pub fn Shell(
    user: Option<Me>,
    sidebar: Option<Element>,
    actions: Option<Element>,
    search: Option<Element>,
    children: Element,
) -> Element {
    let mut drawer_open = use_signal(|| false);
    let mut toggle = use_signal(|| None::<Rc<MountedData>>);
    let mut drawer = use_signal(|| None::<Rc<MountedData>>);
    let route = use_route::<Route>();

    use_effect(use_reactive!(|route| {
        let _ = route;
        if *drawer_open.peek() {
            drawer_open.set(false);
        }
    }));

    // Focus moves into the drawer once it is visible, so Escape reaches it.
    use_effect(move || {
        if drawer_open()
            && let Some(panel) = drawer.peek().clone()
        {
            spawn(async move {
                let _ = panel.set_focus(true).await;
            });
        }
    });

    // The toggle is inert while the drawer is open, so it can only take focus after the render.
    let mut refocus_toggle = use_signal(|| false);
    use_effect(move || {
        if refocus_toggle() && !drawer_open() {
            refocus_toggle.set(false);
            if let Some(button) = toggle.peek().clone() {
                spawn(async move {
                    let _ = button.set_focus(true).await;
                });
            }
        }
    });

    let mut close_and_refocus = move || {
        drawer_open.set(false);
        refocus_toggle.set(true);
    };

    let has_sidebar = sidebar.is_some();
    // With a sidebar the content scrolls inside the frame, so lists can size themselves to it.
    let frame_height = if has_sidebar {
        "h-dvh"
    } else {
        "min-h-screen md:h-screen md:min-h-0"
    };
    let aside_class = if drawer_open() {
        "fixed inset-y-0 left-0 z-30 flex w-64 shadow-xl"
    } else {
        "hidden"
    };

    rsx! {
        div { class: "flex {frame_height} flex-col bg-slate-50 text-gray-800 dark:bg-slate-950 dark:text-slate-200",
            header {
                inert: if drawer_open() { "true" },
                class: "relative flex h-12 shrink-0 items-center gap-2 border-b border-slate-200 bg-white px-4 dark:border-slate-800 dark:bg-slate-900",
                if has_sidebar {
                    button {
                        r#type: "button",
                        class: "-ml-1.5 inline-flex size-8 items-center justify-center rounded-md text-slate-600 hover:bg-slate-100 focus-visible:outline-2 focus-visible:outline-indigo-600 md:hidden dark:text-slate-300 dark:hover:bg-slate-800",
                        aria_label: t!("sidebar-open"),
                        aria_expanded: drawer_open(),
                        aria_controls: SIDEBAR_ID,
                        onmounted: move |event| toggle.set(Some(event.data())),
                        onclick: move |_| drawer_open.set(true),
                        MenuIcon {}
                    }
                }
                span { class: "inline-block size-3 rounded-sm bg-indigo-600 dark:bg-indigo-400" }
                span { class: "font-bold text-gray-900 dark:text-white", {t!("app-name")} }
                div { class: "ml-auto flex items-center gap-1 sm:gap-3",
                    if let Some(search) = search {
                        {search}
                    }
                    if let Some(actions) = actions {
                        {actions}
                    }
                    if let Some(user) = user {
                        UserMenu { user }
                    }
                }
            }
            if let Some(sidebar) = sidebar {
                div { class: "flex min-h-0 flex-1",
                    if drawer_open() {
                        // Backdrop: a click outside the drawer closes it.
                        div {
                            class: "fixed inset-0 z-20 bg-slate-900/40 md:hidden",
                            onclick: move |_| drawer_open.set(false),
                        }
                    }
                    aside {
                        id: SIDEBAR_ID,
                        tabindex: "-1",
                        role: if drawer_open() { "dialog" },
                        aria_modal: if drawer_open() { "true" },
                        aria_label: t!("sidebar-volumes"),
                        class: "{aside_class} flex-col overflow-y-auto border-r border-slate-200 bg-white p-3 focus:outline-none md:static md:z-auto md:flex md:w-56 md:shadow-none dark:border-slate-800 dark:bg-slate-900",
                        onmounted: move |event| drawer.set(Some(event.data())),
                        onkeydown: move |event| {
                            if drawer_open() && event.key() == Key::Escape {
                                close_and_refocus();
                            }
                        },
                        {sidebar}
                    }
                    main {
                        inert: if drawer_open() { "true" },
                        class: "min-w-0 flex-1 overflow-y-auto p-4 md:p-6",
                        {children}
                    }
                }
            } else {
                main { class: "flex flex-1 items-center justify-center p-6", {children} }
            }
        }
    }
}
