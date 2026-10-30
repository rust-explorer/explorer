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
pub fn FolderPlusIcon(#[props(default = "size-3.5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z" }
            path { d: "M12 11v5M9.5 13.5h5" }
        }
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

#[component]
pub fn MoreIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            circle { cx: "5", cy: "12", r: "1" }
            circle { cx: "12", cy: "12", r: "1" }
            circle { cx: "19", cy: "12", r: "1" }
        }
    }
}

#[component]
pub fn MoveIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class, path { d: "M4 12h13m0 0-5-5m5 5-5 5M20 5v14" } }
    }
}

#[component]
pub fn CopyIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            rect { x: "9", y: "9", width: "11", height: "11", rx: "2" }
            path { d: "M5 15V6a2 2 0 0 1 2-2h8" }
        }
    }
}

#[component]
pub fn TrashIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M4 7h16M10 11v6M14 11v6M6 7l1 12a2 2 0 0 0 2 2h6a2 2 0 0 0 2-2l1-12M9 7V4h6v3" }
        }
    }
}

#[component]
pub fn RefreshIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M20 11a8 8 0 0 0-14.9-3M4 5v3h3M4 13a8 8 0 0 0 14.9 3M20 19v-3h-3" }
        }
    }
}

#[component]
pub fn StarIcon(
    #[props(default = "size-5".to_owned())] class: String,
    #[props(default)] filled: bool,
) -> Element {
    let fill = if filled { "currentColor" } else { "none" };
    rsx! {
        Icon { class,
            path {
                fill,
                d: "m12 3 2.7 5.6 6.1.9-4.4 4.3 1 6.1L12 17l-5.4 2.9 1-6.1L3.2 9.5l6.1-.9z",
            }
        }
    }
}

#[component]
pub fn SearchIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M10.5 18a7.5 7.5 0 1 0 0-15 7.5 7.5 0 0 0 0 15ZM16 16l5 5" }
        }
    }
}

#[component]
pub fn ArrowLeftIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M19 12H5M11 6l-6 6 6 6" }
        }
    }
}

#[component]
pub fn ListIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M9 6h11M9 12h11M9 18h11M4 6h.01M4 12h.01M4 18h.01" }
        }
    }
}

#[component]
pub fn GridIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M4 4h6v6H4zM14 4h6v6h-6zM4 14h6v6H4zM14 14h6v6h-6z" }
        }
    }
}

#[component]
pub fn ChartIcon(#[props(default = "size-5".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M21 12a9 9 0 1 1-9-9v9zM15 3.5A9 9 0 0 1 20.5 9H15z" }
        }
    }
}

#[component]
pub fn WarningIcon(#[props(default = "size-4".to_owned())] class: String) -> Element {
    rsx! {
        Icon { class,
            path { d: "M12 4 2.5 20h19zM12 10v4M12 17h.01" }
        }
    }
}

/// The app's mark, a file tree drawing an E; colors inverted in dark mode. Also in
/// `public/favicon.svg`.
#[component]
pub fn Logo(#[props(default = "size-6".to_owned())] class: String) -> Element {
    rsx! {
        svg { class, view_box: "0 0 64 64", "aria-hidden": "true",
            rect {
                class: "fill-slate-900 dark:fill-white",
                x: "2",
                y: "2",
                width: "60",
                height: "60",
                rx: "14",
            }
            path {
                class: "stroke-white dark:stroke-slate-900",
                d: "M36 15H23a4.5 4.5 0 0 0-4.5 4.5V42a6 6 0 0 0 6 6H36M18.5 31.5H36",
                fill: "none",
                stroke_width: "5",
                stroke_linecap: "round",
                stroke_linejoin: "round",
            }
            g { class: "fill-indigo-400 dark:fill-indigo-600",
                circle { cx: "44.5", cy: "15", r: "4.5" }
                circle { cx: "44.5", cy: "31.5", r: "4.5" }
                circle { cx: "44.5", cy: "48", r: "4.5" }
            }
        }
    }
}
