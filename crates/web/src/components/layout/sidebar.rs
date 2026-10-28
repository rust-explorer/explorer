use dioxus::prelude::*;
use dioxus_i18n::prelude::i18n;
use dioxus_i18n::t;
use shared::api::browse::{Space, VolumeInfo};

use crate::components::common::icons::{LockIcon, VolumeIcon};
use crate::components::favorites::FavoritesList;
use crate::format::format_size;
use crate::i18n::attr;
use crate::routes::{Route, Segments};

/// Share of the volume in use, in whole percent.
pub fn used_percent(space: &Space) -> u8 {
    if space.total == 0 {
        return 0;
    }
    let used = u128::from(space.total.saturating_sub(space.available));
    let percent = used * 100 / u128::from(space.total);
    u8::try_from(percent).unwrap_or(100)
}

#[component]
pub fn Sidebar(volumes: Vec<VolumeInfo>, current: String) -> Element {
    let space = volumes
        .iter()
        .find(|volume| volume.name == current)
        .and_then(|volume| volume.space.clone());

    rsx! {
        nav { class: "flex flex-1 flex-col gap-1", aria_label: t!("sidebar-volumes"),
            h2 { class: "mx-1 mb-1 mt-1.5 text-[11px] font-medium uppercase tracking-wider text-slate-400 dark:text-slate-500",
                {t!("sidebar-volumes")}
            }
            ul { class: "flex flex-col gap-0.5",
                for volume in volumes {
                    li { key: "{volume.name}",
                        VolumeLink { active: volume.name == current, volume }
                    }
                }
            }
        }
        FavoritesList {}
        if let Some(space) = space {
            FreeSpace { volume: current, space }
        }
    }
}

#[component]
fn VolumeLink(volume: VolumeInfo, active: bool) -> Element {
    let class = if active {
        "flex items-center gap-2 rounded-md bg-indigo-50 px-2 py-1.5 text-sm font-semibold text-indigo-700 dark:bg-indigo-500/15 dark:text-indigo-300"
    } else {
        "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm text-slate-700 hover:bg-slate-100 dark:text-slate-300 dark:hover:bg-slate-800"
    };
    let read_only = t!("sidebar-read-only");

    rsx! {
        Link {
            to: Route::Browse {
                volume: volume.name.clone().into(),
                path: Segments::default(),
            },
            class,
            aria_current: if active { "page" },
            VolumeIcon { class: "size-4 shrink-0" }
            span { class: "truncate", {volume.name} }
            if volume.read_only {
                span {
                    class: "ml-auto text-slate-400 dark:text-slate-500",
                    role: "img",
                    title: "{read_only}",
                    aria_label: "{read_only}",
                    LockIcon {}
                }
            }
        }
    }
}

#[component]
fn FreeSpace(volume: String, space: Space) -> Element {
    let locale = i18n().language();
    let percent = used_percent(&space);
    let available = format_size(space.available, &locale);
    let used = format_size(space.total.saturating_sub(space.available), &locale);
    let total = format_size(space.total, &locale);

    rsx! {
        div { class: "mt-4 px-1 text-[11px] text-slate-400 dark:text-slate-500",
            p { class: "truncate", {t!("sidebar-free", volume: volume, available: available)} }
            div {
                class: "mt-1 h-1 rounded-full bg-slate-200 dark:bg-slate-700",
                role: "meter",
                aria_label: t!("sidebar-used-label"),
                aria_valuemin: "0",
                aria_valuemax: "100",
                aria_valuenow: "{percent}",
                aria_valuetext: attr(t!("sidebar-used", used: used, total: total)),
                div {
                    class: "h-1 rounded-full bg-indigo-300 dark:bg-indigo-400",
                    width: "{percent}%",
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn used_percent_of_a_volume() {
        let space = |total, available| Space { total, available };
        assert_eq!(used_percent(&space(1000, 320)), 68);
        assert_eq!(used_percent(&space(1000, 1000)), 0);
        assert_eq!(used_percent(&space(1000, 0)), 100);
        assert_eq!(used_percent(&space(0, 0)), 0);
        assert_eq!(used_percent(&space(u64::MAX, 0)), 100);
        assert_eq!(used_percent(&space(10, 20)), 0);
    }
}
