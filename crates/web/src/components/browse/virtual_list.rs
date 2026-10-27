use std::rc::Rc;

use dioxus::html::geometry::PixelsVector2D;
use dioxus::prelude::*;

use crate::listing::{reveal_offset, visible_range};

const OVERSCAN: usize = 10;

/// A scroll container that only renders the rows near its viewport, between two spacers.
/// It scrolls row `reveal` into view whenever that changes.
#[component]
pub fn VirtualList(
    total: usize,
    row_height: f64,
    row: Callback<usize, Element>,
    #[props(default)] reveal: Option<usize>,
    #[props(extends = GlobalAttributes, extends = div)] attributes: Vec<Attribute>,
) -> Element {
    let mut scroll_top = use_signal(|| 0.0);
    let mut viewport = use_signal(|| 0.0);
    let mut container = use_signal(|| None::<Rc<MountedData>>);
    use_effect(use_reactive!(|reveal| {
        let (Some(index), Some(container)) = (reveal, container()) else {
            return;
        };
        spawn(async move {
            let Ok(offset) = container.get_scroll_offset().await else {
                return;
            };
            if let Some(top) = reveal_offset(index, row_height, offset.y, *viewport.peek()) {
                let _ = container
                    .scroll(PixelsVector2D::new(0.0, top), ScrollBehavior::Instant)
                    .await;
            }
        });
    }));
    let range_at =
        move |top: f64, height: f64| visible_range(top, height, row_height, total, OVERSCAN);
    let range = range_at(scroll_top(), viewport());
    #[expect(clippy::cast_precision_loss, reason = "row counts stay far below 2^52")]
    let (top_space, bottom_space) = (
        range.start as f64 * row_height,
        (total - range.end) as f64 * row_height,
    );

    rsx! {
        div {
            onmounted: move |event| container.set(Some(event.data())),
            // Scroll events fire every frame; only a change of rendered rows needs a render.
            onscroll: move |event| {
                let top = event.data().scroll_top();
                if range_at(top, *viewport.peek()) != range_at(*scroll_top.peek(), *viewport.peek()) {
                    scroll_top.set(top);
                }
            },
            onresize: move |event| {
                if let Ok(size) = event.data().get_content_box_size() {
                    viewport.set(size.height);
                }
            },
            ..attributes,
            div { aria_hidden: "true", height: "{top_space}px" }
            for index in range {
                Fragment { key: "{index}", {row.call(index)} }
            }
            div { aria_hidden: "true", height: "{bottom_space}px" }
        }
    }
}
