use dioxus::prelude::*;

const MENU: &str = "w-44 rounded-lg border border-slate-200 bg-white p-1 shadow-lg dark:border-slate-700 dark:bg-slate-900";
const ITEM: &str = "block w-full rounded-md px-3 py-1.5 text-left text-sm hover:bg-slate-100 focus-visible:bg-slate-100 focus-visible:outline-none dark:hover:bg-slate-800 dark:focus-visible:bg-slate-800";
const ITEM_NORMAL: &str = "text-slate-700 dark:text-slate-200";
const ITEM_DANGER: &str = "text-red-700 dark:text-red-400";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum NavKey {
    Up,
    Down,
    Home,
    End,
}

impl NavKey {
    fn from_key(key: &Key) -> Option<Self> {
        match key {
            Key::ArrowUp => Some(Self::Up),
            Key::ArrowDown => Some(Self::Down),
            Key::Home => Some(Self::Home),
            Key::End => Some(Self::End),
            _ => None,
        }
    }
}

/// Index to focus after `key`, with Up and Down wrapping around.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
fn next_index(current: usize, len: usize, key: NavKey) -> usize {
    match key {
        NavKey::Down => (current + 1) % len,
        NavKey::Up => (current + len - 1) % len,
        NavKey::Home => 0,
        NavKey::End => len - 1,
    }
}

/// A `role="menu"` popup named by `label`. The caller places it through `class`, or at the
/// client coordinates `at` (set through CSSOM, which the CSP allows, unlike a `style` attribute).
/// `on_close(true)` asks the caller to hand focus back to the trigger; `false` means focus
/// already moved outside the menu.
#[component]
pub fn Menu(
    id: String,
    class: String,
    label: String,
    #[props(default)] at: Option<(f64, f64)>,
    on_close: EventHandler<bool>,
    children: Element,
) -> Element {
    let mut menu = use_signal(|| None::<std::rc::Rc<MountedData>>);

    rsx! {
        // Transparent backdrop: a click anywhere outside the menu closes it.
        div {
            class: "fixed inset-0 z-20",
            onclick: move |_| on_close.call(true),
        }
        div {
            id,
            role: "menu",
            aria_label: label,
            class: "{class} {MENU}",
            left: at.map(|(x, _)| format!("{x}px")),
            top: at.map(|(_, y)| format!("{y}px")),
            onmounted: move |event| {
                focus_item(&event.data(), None);
                menu.set(Some(event.data()));
            },
            onkeydown: move |event| {
                let key = event.key();
                if key == Key::Escape {
                    on_close.call(true);
                } else if key == Key::Tab {
                    event.prevent_default();
                    on_close.call(true);
                } else if let Some(nav) = NavKey::from_key(&key) {
                    event.prevent_default();
                    if let Some(menu) = menu() {
                        focus_item(&menu, Some(nav));
                    }
                }
            },
            onfocusout: move |event| {
                if let Some(menu) = menu()
                    && focus_left(&menu, &event)
                {
                    on_close.call(false);
                }
            },
            {children}
        }
    }
}

#[component]
pub fn MenuItem(
    on_select: EventHandler<()>,
    #[props(default)] danger: bool,
    children: Element,
) -> Element {
    let tone = if danger { ITEM_DANGER } else { ITEM_NORMAL };
    rsx! {
        button {
            r#type: "button",
            role: "menuitem",
            tabindex: "-1",
            class: "{ITEM} {tone}",
            onclick: move |_| on_select.call(()),
            {children}
        }
    }
}

/// Focuses the first item, or the one `key` leads to from the focused item.
#[cfg(target_arch = "wasm32")]
fn focus_item(menu: &MountedData, key: Option<NavKey>) {
    use wasm_bindgen::JsCast as _;

    let Some(menu) = menu.downcast::<web_sys::Element>() else {
        return;
    };
    let Ok(list) = menu.query_selector_all("[role=menuitem]") else {
        return;
    };
    let items: Vec<web_sys::HtmlElement> = (0..list.length())
        .filter_map(|i| list.item(i)?.dyn_into().ok())
        .collect();
    if items.is_empty() {
        return;
    }
    let target = key.map_or(0, |key| {
        let active = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.active_element());
        let current = items
            .iter()
            .position(|item| active.as_ref() == Some(item.as_ref()))
            .unwrap_or(0);
        next_index(current, items.len(), key)
    });
    let _ = items[target].focus();
}

#[cfg(not(target_arch = "wasm32"))]
fn focus_item(_: &MountedData, _: Option<NavKey>) {}

/// Whether focus went to an element outside the menu (not just nowhere, as when it is removed).
#[cfg(target_arch = "wasm32")]
fn focus_left(menu: &MountedData, event: &FocusEvent) -> bool {
    use wasm_bindgen::JsCast as _;

    let Some(menu) = menu.downcast::<web_sys::Element>() else {
        return false;
    };
    event
        .data()
        .downcast::<web_sys::FocusEvent>()
        .and_then(web_sys::FocusEvent::related_target)
        .and_then(|target| target.dyn_into::<web_sys::Node>().ok())
        .is_some_and(|target| !menu.contains(Some(&target)))
}

#[cfg(not(target_arch = "wasm32"))]
fn focus_left(_: &MountedData, _: &FocusEvent) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_wraps_and_jumps() {
        assert_eq!(next_index(0, 3, NavKey::Down), 1);
        assert_eq!(next_index(2, 3, NavKey::Down), 0);
        assert_eq!(next_index(0, 3, NavKey::Up), 2);
        assert_eq!(next_index(1, 3, NavKey::Home), 0);
        assert_eq!(next_index(1, 3, NavKey::End), 2);
    }
}
