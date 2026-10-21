use dioxus::prelude::*;

use crate::listing::visible_range;

const OVERSCAN: usize = 10;

/// A scroll container that only renders the rows near its viewport, between two spacers.
#[component]
pub fn VirtualList(
    total: usize,
    row_height: f64,
    row: Callback<usize, Element>,
    #[props(extends = GlobalAttributes, extends = div)] attributes: Vec<Attribute>,
) -> Element {
    let mut scroll_top = use_signal(|| 0.0);
    let mut viewport = use_signal(|| 0.0);
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
