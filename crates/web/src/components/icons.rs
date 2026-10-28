use dioxus::prelude::*;

#[component]
fn Icon(class: String, children: Element) -> Element {
    rsx! {
        svg {
            class,
            view_box: "0 0 24 24",
            fill: "none",
            stroke: "currentColor",
            stroke_width: "1.75",
            stroke_linecap: "round",
            stroke_linejoin: "round",
            "aria-hidden": "true",
            {children}
        }
    }
}

#[component]
pub fn MenuIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class, path { d: "M4 6h16M4 12h16M4 18h16" } }
    }
}

#[component]
pub fn VolumeIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            rect { x: "3", y: "4", width: "18", height: "7", rx: "2" }
            rect { x: "3", y: "13", width: "18", height: "7", rx: "2" }
            path { d: "M7 7.5h.01M7 16.5h.01" }
        }
    }
}

#[component]
pub fn LockIcon(#[props(default = "size-3.5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            rect { x: "5", y: "11", width: "14", height: "10", rx: "2" }
            path { d: "M8 11V7a4 4 0 0 1 8 0v4" }
        }
    }
}

#[component]
pub fn FolderIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" }
        }
    }
}

#[component]
pub fn FileIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" }
            path { d: "M14 3v5h5" }
        }
    }
}
