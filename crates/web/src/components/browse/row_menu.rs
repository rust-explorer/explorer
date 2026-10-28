use std::rc::Rc;

use dioxus::prelude::*;
use dioxus_i18n::t;
use shared::api::browse::Entry;

use crate::actions::{RowAction, row_actions};
use crate::components::common::{FocusTarget, Menu, MenuItem};

pub const ROW_MENU_ID: &str = "row-menu";

/// An open row menu: its entry, where it opens, and the ⋯ button that takes focus back.
#[derive(Clone, PartialEq)]
pub struct MenuRequest {
    pub entry: Entry,
    pub at: MenuAt,
    pub trigger: Option<FocusTarget>,
}

/// Where a row menu opens: at the pointer for a right click, under the ⋯ button otherwise.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum MenuAt {
    Pointer { x: f64, y: f64 },
    Below(Anchor),
}

/// A button's client rectangle.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Anchor {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}

/// Where the menu of a ⋯ `button` opens: under it, or where `event` clicked when it can't be
/// measured.
pub async fn below_button(button: Option<Rc<MountedData>>, event: &MouseEvent) -> MenuAt {
    let point = event.client_coordinates();
    let fallback = MenuAt::Pointer {
        x: point.x,
        y: point.y,
    };
    let Some(button) = button else {
        return fallback;
    };
    button.get_client_rect().await.map_or(fallback, |rect| {
        MenuAt::Below(Anchor {
            left: rect.min_x(),
            top: rect.min_y(),
            right: rect.max_x(),
            bottom: rect.max_y(),
        })
    })
}

/// A right click opens the menu at the pointer.
pub fn at_pointer(event: &MouseEvent) -> MenuAt {
    let point = event.client_coordinates();
    MenuAt::Pointer {
        x: point.x,
        y: point.y,
    }
}

const MENU_WIDTH: f64 = 176.0;
const ITEM_HEIGHT: f64 = 32.0;
const MENU_PADDING: f64 = 10.0;
const MARGIN: f64 = 8.0;
const GAP: f64 = 4.0;

/// The menu's height before it can be measured.
fn estimated_height(items: usize) -> f64 {
    #[expect(clippy::cast_precision_loss, reason = "a menu has a handful of items")]
    let height = items as f64 * ITEM_HEIGHT + MENU_PADDING;
    height
}

/// The top-left corner of a menu `height` high in client coordinates, kept inside a `viewport`
/// of (width, height).
fn menu_origin(at: MenuAt, height: f64, viewport: (f64, f64)) -> (f64, f64) {
    let (x, y) = match at {
        MenuAt::Pointer { x, y } => (x, y),
        MenuAt::Below(anchor) => {
            let below = anchor.bottom + GAP;
            let y = if below + height > viewport.1 - MARGIN {
                anchor.top - height - GAP
            } else {
                below
            };
            (anchor.right - MENU_WIDTH, y)
        }
    };
    let clamp = |value: f64, size: f64, limit: f64| value.min(limit - size - MARGIN).max(MARGIN);
    (
        clamp(x, MENU_WIDTH, viewport.0),
        clamp(y, height, viewport.1),
    )
}

#[component]
pub fn RowMenu(
    request: MenuRequest,
    read_only: bool,
    on_close: EventHandler<bool>,
    on_action: EventHandler<RowAction>,
) -> Element {
    let actions = row_actions(request.entry.kind, read_only);
    let mut measured = use_signal(|| None::<f64>);
    let height = measured().unwrap_or_else(|| estimated_height(actions.len()));
    let at = menu_origin(request.at, height, viewport());

    rsx! {
        Menu {
            id: ROW_MENU_ID,
            class: "fixed z-20",
            label: t!("row-menu-open", name: request.entry.name.clone()),
            at,
            on_close,
            on_mounted: move |menu: Rc<MountedData>| async move {
                if let Ok(rect) = menu.get_client_rect().await {
                    measured.set(Some(rect.height()));
                }
            },
            for action in actions {
                MenuItem {
                    key: "{action:?}",
                    danger: action == RowAction::Delete,
                    on_select: move |()| on_action.call(action),
                    {action_label(action)}
                }
            }
        }
    }
}

pub fn action_label(action: RowAction) -> String {
    match action {
        RowAction::Download => t!("action-download"),
        RowAction::Rename => t!("action-rename"),
        RowAction::MoveTo => t!("action-move-to"),
        RowAction::CopyTo => t!("action-copy-to"),
        RowAction::AnalyzeUsage => t!("action-analyze-usage"),
        RowAction::CalculateSize => t!("action-calculate-size"),
        RowAction::Delete => t!("action-delete"),
    }
}

#[cfg(target_arch = "wasm32")]
fn viewport() -> (f64, f64) {
    let size = |value: Result<wasm_bindgen::JsValue, _>| value.ok().and_then(|v| v.as_f64());
    web_sys::window()
        .and_then(|w| Some((size(w.inner_width())?, size(w.inner_height())?)))
        .unwrap_or((f64::INFINITY, f64::INFINITY))
}

#[cfg(not(target_arch = "wasm32"))]
fn viewport() -> (f64, f64) {
    (f64::INFINITY, f64::INFINITY)
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEWPORT: (f64, f64) = (1000.0, 800.0);

    #[test]
    fn opens_at_the_pointer() {
        assert_eq!(
            menu_origin(
                MenuAt::Pointer { x: 100.0, y: 200.0 },
                estimated_height(2),
                VIEWPORT
            ),
            (100.0, 200.0)
        );
    }

    #[test]
    fn pointer_menu_stays_in_the_viewport() {
        // Two items: 74px high.
        assert_eq!(
            menu_origin(
                MenuAt::Pointer { x: 990.0, y: 790.0 },
                estimated_height(2),
                VIEWPORT
            ),
            (1000.0 - 176.0 - 8.0, 800.0 - 74.0 - 8.0)
        );
        assert_eq!(
            menu_origin(
                MenuAt::Pointer { x: -5.0, y: 0.0 },
                estimated_height(2),
                VIEWPORT
            ),
            (8.0, 8.0)
        );
    }

    #[test]
    fn button_menu_aligns_under_its_right_edge() {
        let anchor = Anchor {
            left: 900.0,
            top: 100.0,
            right: 932.0,
            bottom: 132.0,
        };
        assert_eq!(
            menu_origin(MenuAt::Below(anchor), estimated_height(2), VIEWPORT),
            (932.0 - 176.0, 136.0)
        );
    }

    #[test]
    fn button_menu_flips_above_near_the_bottom() {
        let anchor = Anchor {
            left: 900.0,
            top: 750.0,
            right: 932.0,
            bottom: 782.0,
        };
        assert_eq!(
            menu_origin(MenuAt::Below(anchor), estimated_height(2), VIEWPORT),
            (932.0 - 176.0, 750.0 - 74.0 - 4.0)
        );
    }

    #[test]
    fn measured_height_decides_the_flip() {
        let anchor = Anchor {
            left: 900.0,
            top: 600.0,
            right: 932.0,
            bottom: 632.0,
        };
        assert_eq!(
            menu_origin(MenuAt::Below(anchor), estimated_height(2), VIEWPORT),
            (932.0 - 176.0, 636.0)
        );
        assert_eq!(
            menu_origin(MenuAt::Below(anchor), 200.0, VIEWPORT),
            (932.0 - 176.0, 600.0 - 200.0 - 4.0)
        );
    }

    #[test]
    fn tiny_viewport_keeps_the_margin() {
        assert_eq!(
            menu_origin(
                MenuAt::Pointer { x: 50.0, y: 50.0 },
                estimated_height(5),
                (100.0, 100.0)
            ),
            (8.0, 8.0)
        );
    }
}
