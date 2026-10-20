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

#[component]
pub fn VideoIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            rect { x: "3", y: "5", width: "18", height: "14", rx: "2" }
            path { d: "m10 9 5 3-5 3z" }
        }
    }
}

#[component]
pub fn ImageIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            rect { x: "3", y: "4", width: "18", height: "16", rx: "2" }
            circle { cx: "9", cy: "10", r: "1.5" }
            path { d: "m21 16-4.5-4.5L7 20" }
        }
    }
}

#[component]
pub fn AudioIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M9 18V6l11-2v12" }
            circle { cx: "6", cy: "18", r: "3" }
            circle { cx: "17", cy: "16", r: "3" }
        }
    }
}

#[component]
pub fn ArchiveIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            rect { x: "3", y: "4", width: "18", height: "5", rx: "1" }
            path { d: "M5 9v9a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V9M10 13h4" }
        }
    }
}

#[component]
pub fn DocumentIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M14 3H7a2 2 0 0 0-2 2v14a2 2 0 0 0 2 2h10a2 2 0 0 0 2-2V8z" }
            path { d: "M14 3v5h5M9 13h6M9 17h6" }
        }
    }
}

#[component]
pub fn DownloadIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M12 4v11m0 0-4-4m4 4 4-4M5 20h14" }
        }
    }
}

#[component]
pub fn ArrowUpIcon(#[props(default = "size-3.5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M12 19V5m0 0-6 6m6-6 6 6" }
        }
    }
}

#[component]
pub fn ArrowDownIcon(#[props(default = "size-3.5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M12 5v14m0 0-6-6m6 6 6-6" }
        }
    }
}

#[component]
pub fn PlusIcon(#[props(default = "size-6".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class, path { d: "M12 5v14M5 12h14" } }
    }
}

#[component]
pub fn MinusIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class, path { d: "M5 12h14" } }
    }
}

#[component]
pub fn CloseIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class, path { d: "M6 6l12 12M18 6 6 18" } }
    }
}

#[component]
pub fn CheckIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class, path { d: "m5 12 5 5L20 7" } }
    }
}
