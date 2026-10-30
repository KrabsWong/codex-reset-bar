//! The panel's UI: one component, running once, whose texts and rows bind to
//! signals the tasks write.
//!
//! Layout (360 × 728): header, the latest reset, announced upcoming resets, compact statistics, the announcement history, and the API's
//! attribution.

use std::sync::{Mutex, OnceLock};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use nsh_base::{button, theme, tooltip};
use nsh_core::ui::prelude::*;

use crate::{api, cache, calendar, format, model::*, settings, i18n::{self, tr}};

/// Inner margin. The popover clips its content to a rounded shape with an
/// arrow on top, so the panel keeps a margin larger than both: nothing of it
/// is cut at the corners.
const PAD: f32 = 14.0;
/// Above the header, past the arrow.
const TOP: f32 = 14.0;
/// Bottom inset for the attribution footer.
const BOTTOM: f32 = 12.0;
/// How long the refresh stays visible at the least, however fast the answer.
const MIN_BUSY: Duration = Duration::from_millis(700);

/// How many announcements one page holds, and how often the status is read
/// again in the background.
const PAGE: u32 = 30;
/// How often an open panel looks at the clock, and how old the status may be
/// before it is read again.
const TICK: Duration = Duration::from_secs(15);
const STALE: i64 = 60;

/// Panel visibility controls the detailed history refresh; status monitoring
/// continues independently in the native host.
static VISIBLE: AtomicBool = AtomicBool::new(false);
static SETTINGS_EVENTS: OnceLock<async_channel::Sender<()>> = OnceLock::new();
static SHOW_FRAME: OnceLock<async_channel::Sender<Option<i64>>> = OnceLock::new();

static OPEN: OnceLock<fn(&str)> = OnceLock::new();
static CLOSE: OnceLock<fn()> = OnceLock::new();
static AVATAR_EVENTS: OnceLock<async_channel::Sender<Option<Vec<u8>>>> = OnceLock::new();
static UPDATE: OnceLock<fn()> = OnceLock::new();
// Keep the latest value even before the lazy popover is mounted.
static UPDATE_STATE: Mutex<(Option<String>, Option<async_channel::Sender<Option<String>>>)> = Mutex::new((None, None));
pub fn set_update_handler(handler: fn()) { let _ = UPDATE.set(handler); }
pub fn update_available(version: Option<String>) {
    let mut state = UPDATE_STATE.lock().unwrap();
    state.0 = version.clone();
    if let Some(sender) = &state.1 { let _ = sender.try_send(version); }
}
fn show_update() {
    close();
    if let Some(handler) = UPDATE.get() { handler(); }
}
fn update_label(version: Option<String>) -> String {
    format!("{} v{}", tr("Update to", "更新至"), version.unwrap_or_default())
}

pub fn avatar_changed(bytes: Option<Vec<u8>>) {
    if let Some(sender) = AVATAR_EVENTS.get() { let _ = sender.try_send(bytes); }
}
fn avatar_source(bytes: Option<Vec<u8>>) -> String {
    let (mime, bytes) = bytes.map(|bytes| ("image/png", bytes))
        .unwrap_or_else(|| ("image/jpeg", include_bytes!("../resources/codex-resets-logo.jpg").to_vec()));
    format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
}

/// Notify the UI engine after a preference changes on AppKit's main thread.
pub fn refresh_language() {
    if let Some(sender) = SETTINGS_EVENTS.get() { let _ = sender.try_send(()); }
}

/// Tells the panel it is on screen (the host calls this as the popover
/// opens and closes). The lightweight background monitor remains active.
pub fn set_visible(visible: bool) {
    let seen = crate::monitor::seen_at();
    VISIBLE.store(visible, Ordering::Relaxed);
    crate::monitor::set_visible(visible);
    if visible {
        if let Some(sender) = SHOW_FRAME.get() {
            let _ = sender.try_send(seen);
        }
    }
}

/// How the UI opens a link outside the app (set by `main`).
pub fn set_open_handler(handler: fn(&str)) {
    let _ = OPEN.set(handler);
}

/// How the UI closes the panel (set by `main`): a link leaves it as the
/// browser comes forward.
pub fn set_close_handler(handler: fn()) {
    let _ = CLOSE.set(handler);
}

fn close() {
    if let Some(handler) = CLOSE.get() {
        handler();
    }
}

fn open(url: impl Into<String>) {
    let url = url.into();
    if let Some(handler) = OPEN.get() {
        handler(&url);
        close();
    }
}

/// Everything the panel reads: handles, so the tasks write and the views
/// follow.
#[derive(Clone, Copy)]
struct State {
    status: Signal<Option<StatusData>>,
    // Preserve the pre-open acknowledgment while this panel is being viewed.
    unread_since: Signal<Option<i64>>,
    history: Signal<Vec<Reset>>,
    cursor: Signal<Option<String>>,
    has_more: Signal<bool>,
    error: Signal<Option<String>>,
    loading: Signal<bool>,
    busy: Signal<bool>,
    now: Signal<i64>,
    updated_at: Signal<Option<i64>>,
    /// 0.4…0.9, for the skeleton's breathing.
    pulse: Signal<f32>,
    /// Set for a moment after a refresh succeeds, so a fast answer still
    /// shows that something happened.
    flash: Signal<bool>,
    avatar: Signal<String>,
    update_version: Signal<Option<String>>,
}

#[component]
pub fn App() -> impl IntoElement {
    i18n::initialize();
    // NSH defers text drawing while detached. A visible popover must request
    // a frame even when cached data has not changed, so deferred draws run.
    let (sender, receiver) = async_channel::bounded(1);
    if VISIBLE.load(Ordering::Relaxed) {
        let _ = sender.try_send(crate::monitor::seen_at());
    }
    let _ = SHOW_FRAME.set(sender);
    // What was read last time paints first: no empty panel, no waiting.
    let (settings_sender, settings_receiver) = async_channel::bounded(1);
    let _ = SETTINGS_EVENTS.set(settings_sender);
    let (update_sender, update_receiver) = async_channel::unbounded();
    let update_version = {
        let mut update = UPDATE_STATE.lock().unwrap();
        update.1 = Some(update_sender);
        update.0.clone()
    };
    let cached = cache::load();
    let now = format::now_epoch();
    let state = State {
        status: signal(cached.as_ref().map(|cache| cache.status.clone())),
        unread_since: signal(crate::monitor::seen_at()),
        history: signal(
            cached
                .as_ref()
                .map(|cache| cache.history.clone())
                .unwrap_or_default(),
        ),
        cursor: signal(cached.as_ref().and_then(|cache| cache.cursor.clone())),
        has_more: signal(cached.as_ref().is_some_and(|cache| cache.has_more)),
        error: signal(Option::<String>::None),
        loading: signal(cached.is_none()),
        busy: signal(false),
        now: signal(now),
        updated_at: signal(cached.as_ref().map(|cache| cache.saved_at)),
        pulse: signal(0.85_f32),
        flash: signal(false),
        avatar: signal(avatar_source(settings::load_avatar())),
        update_version: signal(update_version),
    };

    spawn(async move {
        while let Ok(version) = update_receiver.recv().await {
            state.update_version.set(version);
        }
    });

    spawn(async move {
        while settings_receiver.recv().await.is_ok() {
            i18n::refresh();
        }
    });

    let (avatar_sender, avatar_receiver) = async_channel::unbounded();
    let _ = AVATAR_EVENTS.set(avatar_sender);
    spawn(async move {
        while let Ok(event) = avatar_receiver.recv().await {
            state.avatar.set(avatar_source(event));
        }
    });

    spawn(async move {
        while let Ok(seen) = receiver.recv().await {
            state.unread_since.set(seen);
            request_animation_frame(|_| {});
            // The cached record is already visible, even if refreshing fails.
            if let Some(data) = state.status.get() {
                crate::monitor::displayed(&data);
            }
            // A hidden background check may be newer than the displayed cache.
            // Opening the panel refreshes its full history immediately.
            // Keep open events responsive while a slow refresh is in flight.
            spawn(reload_all(state));
        }
    });

    // Read again when there was nothing to show, or the cache is a minute old.
    let fresh = cached.as_ref().is_some_and(|cache| {
        cache.age(now) < STALE && !cache.has_more
    });
    drop(cached);
    if !fresh {
        spawn(reload_all(state));
    }

    // The skeleton breathes while it waits.
    set_interval(Duration::from_millis(700), move || {
        state
            .pulse
            .update(|value| *value = if *value > 0.6 { 0.4 } else { 0.85 });
    });

    // The clock: relative times and the countdown follow it.
    spawn(async move {
        loop {
            timer(Duration::from_secs(20)).await;
            state.now.set(format::now_epoch());
        }
    });

    // What being open changes: the first look after opening reads everything
    // again when the cache is a minute old; after that only the status, which
    // is the part that moves without the history growing.
    spawn(async move {
        let mut was_visible = false;
        loop {
            timer(TICK).await;
            let visible = VISIBLE.load(Ordering::Relaxed);
            if !visible {
                was_visible = false;
                continue;
            }
            let stale = state
                .updated_at
                .get()
                .is_some_and(|at| format::now_epoch() - at >= STALE);
            if !was_visible {
                was_visible = true;
                if stale {
                    spawn(reload_all(state));
                }
                continue;
            }
            if !stale || state.busy.get() {
                continue;
            }
            if let Ok(data) = api::status().await {
                if state.busy.get() {
                    continue;
                }
                if state.status.get().is_none_or(|previous| {
                    previous.latest_reset != data.latest_reset || previous.stats.total != data.stats.total
                }) {
                    reload_all(state).await;
                    continue;
                }
                crate::monitor::displayed(&data);
                state.status.set(Some(data));
                state.updated_at.set(Some(format::now_epoch()));
                remember(state);
            }
        }
    });

    view()
        .flex_col()
        .w_full()
        .h_full()
        .bg(move || theme().background)
        .text_color(move || theme().foreground)
        .child(header(state))
        .child(
            view()
                .flex_col()
                .flex_1()
                .overflow_hidden()
                .child(
                    view()
                        .flex_col()
                        .gap(px(10.0))
                        .px(px(PAD))
                        .child(
                            when(
                                move || state.status.get().is_some(),
                                move || status_area(state),
                            )
                            .otherwise(move || placeholder(state)),
                        )
                        .child(error_banner(state)),
                )
                .child(when(
                    move || state.status.get().is_some(),
                    move || history_section(state),
                )),
        )
        .child(attribution_footer())
}

// ───────────────────────────── header ─────────────────────────────

fn header(state: State) -> impl IntoElement {
    view()
        .flex_row()
        .items_center()
        .bg(move || tint(blue(), 0.07, 0.14))
        .gap(px(8.0))
        .px(px(PAD))
        .pt(px(TOP))
        .pb(px(12.0))
        .child(
            view()
                .relative()
                .size(px(32.0))
                .child(
                    button()
                        .size(px(32.0))
                        .accessibility_label(move || Some(if state.update_version.get().is_some() {
                            update_label(state.update_version.get()).into()
                        } else { tr("Open Codex Resets website", "打开 Codex Resets 网站").into() }))
                        .on_press(move || {
                            if state.update_version.get().is_some() { show_update(); }
                            else { open(api::SITE); }
                        })
                        .hover(|style| style.opacity(0.75))
                        .child(image(move || state.avatar.get()).size(px(32.0)).rounded(px(16.0)))
                        .child(when(move || state.update_version.get().is_some(), move || {
                            view().absolute().bottom(px(-3.0)).right(px(-3.0))
                                .size(px(18.0)).rounded(px(9.0))
                                .items_center().justify_center().bg(rgb(0x1f6ee8))
                                .tooltip(tooltip().child(text(move || update_label(state.update_version.get()))))
                                .child(text("↑").text_size(px(14.0)).font_weight(FontWeight::BOLD)
                                    .text_color(rgb(0xffffff)))
                        })),
                ),
        )
        .child(
            view()
                .flex_col()
                .flex_1()
                .gap(px(3.0))
                .child(
                    button()
                        .accessibility_label(move || Some(tr("Open Codex Resets website", "打开 Codex Resets 网站").into()))
                        .hover(|style| style.opacity(0.75))
                        .on_press(|| open(api::SITE))
                        .child(
                            view()
                                .child(
                                    text("Codex Resets")
                                        .text_size(px(14.0))
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .text_color(blue),
                                ),
                        ),
                )
                .child(
                    text(move || subtitle(state))
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            button()
                .size(px(30.0))
                .items_center()
                .justify_center()
                .rounded(px(6.0))
                .accessibility_label(move || Some(tr("Refresh", "刷新").into()))
                .disabled(move || state.busy.get())
                .opacity(move || if state.busy.get() { 0.4_f32 } else { 1.0_f32 })
                .hover(|style| style.bg(tint(blue(), 0.12, 0.2)))
                .on_press(move || {
                    if !state.busy.get() {
                        spawn(reload_all(state));
                    }
                })
                .child(
                    view()
                        .tooltip(tooltip().child(text(move || tr("Refresh", "刷新"))))
                        .child(
                            view()
                                .size(px(26.0))
                                .items_center()
                                .justify_center()
                                .origin_center()
                                .style_when(move || state.busy.get(), |style| style.animate_spin())
                                .child(text("↻").text_size(px(22.0)).text_color(blue)),
                        ),
                ),
        )
}

fn subtitle(state: State) -> String {
    if state.busy.get() {
        return tr("Refreshing…", "刷新中…").to_owned();
    }
    if state.flash.get() {
        return match state.updated_at.get() {
            Some(at) => format!("{} · {}", tr("Updated", "已更新"), format::clock_time(at)),
            None => tr("Updated", "已更新").to_owned(),
        };
    }
    if state.status.get().is_none() {
        return if state.error.get().is_some() {
            tr("Unable to connect", "无法连接")
        } else {
            tr("Connecting…", "连接中…")
        }
        .to_owned();
    }
    match state.updated_at.get() {
        Some(at) => format!("{} {}", tr("Updated at", "更新于"), format::clock_time(at)),
        None => "codex-resets.com".to_owned(),
    }
}

fn status_area(state: State) -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(12.0))
        .pt(px(4.0))
        .child(latest_card(state))
        .child(service_description())
        .child(next_card(state))
        .child(watch_row(state))
        .child(stats_row(state))
        .child(reset_calendar(state))
}

fn service_description() -> impl IntoElement {
    text(move || tr("We watch ", "我们关注 "))
        .text_size(px(11.0))
        .text_color(move || theme().muted_foreground)
        .child(
            text("@thsottiaux")
                .text_color(blue)
                .cursor_pointer()
                .on_press(|| open("https://x.com/thsottiaux")),
        )
        .child(text(move || tr(" for Codex reset announcements, so you don't have to.", " 的 Codex 重置公告，为你及时汇总。")))
}

fn placeholder(state: State) -> impl IntoElement {
    when(move || state.loading.get(), move || skeleton(state))
        .otherwise(move || failure_view())
}

/// What waits for the first answer: blocks where the cards and rows will be,
/// breathing together — better than an empty panel that reads as "nothing".
fn skeleton(state: State) -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(14.0))
        .py(px(8.0))
        .child(separator())
        .child(skeleton_row(state))
        .child(skeleton_row(state))
        .child(skeleton_block(state, 180.0, 11.0))
        .child(separator())
        .child(skeleton_row(state))
        .child(skeleton_row(state))
        .child(skeleton_row(state))
}

fn skeleton_row(state: State) -> impl IntoElement {
    view()
        .flex_row()
        .items_start()
        .gap(px(9.0))
        .child(skeleton_block(state, 30.0, 9.0))
        .child(
            view()
                .flex_col()
                .flex_1()
                .gap(px(6.0))
                .child(skeleton_block(state, 260.0, 9.0))
                .child(skeleton_block(state, 44.0, 8.0)),
        )
}

fn skeleton_block(state: State, width: f32, height: f32) -> impl IntoElement {
    view()
        .w(px(width))
        .h(px(height))
        .rounded(px(height / 2.0))
        .bg(move || tint(theme().muted_foreground, 0.2, 0.3))
        .opacity(move || state.pulse.get())
        .transition_opacity()
}

fn failure_view() -> impl IntoElement {
    view()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .h(px(260.0))
        .child(
            text(move || tr("Unable to load data", "暂时无法加载数据"))
                .text_size(px(13.0))
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(|| AdaptiveColor { light: rgb(0x991b1b).into(), dark: rgb(0xfca5a5).into() }),
        )
        .child(
            text(move || tr("Check your connection, then click refresh at the top.", "请检查网络连接，再点击顶部刷新。"))
                .text_size(px(11.0))
                .text_color(move || theme().muted_foreground)
                .number_of_lines(2),
        )
}

fn unread_background() -> AdaptiveColor {
    AdaptiveColor { light: rgb(0xdcfce7).into(), dark: rgb(0x123322).into() }
}

fn unread_foreground() -> AdaptiveColor {
    AdaptiveColor { light: rgb(0x166534).into(), dark: rgb(0x86efac).into() }
}

fn in_unread_range(seen: Option<i64>, at: Option<i64>, latest: Option<i64>) -> bool {
    matches!((seen, at, latest), (Some(seen), Some(at), Some(latest)) if at > seen && at <= latest)
}

fn unread_record(state: State, reset: &Reset) -> bool {
    in_unread_range(
        state.unread_since.get(),
        format::parse(&reset.announced_at),
        latest(state).and_then(|reset| format::parse(&reset.announced_at)),
    )
}

fn latest_card(state: State) -> impl IntoElement {
    view()
        .flex_col()
        .gap(px(5.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || tr("Latest reset", "最近一次重置"))
                        .text_size(px(14.0))
                        .text_color(move || theme().foreground)
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(view().flex_1())
                .child(
                    text(move || {
                        latest(state)
                            .and_then(|reset| format::parse(&reset.announced_at))
                            .map(|at| format::relative(at, state.now.get()))
                            .unwrap_or_else(|| tr("No resets yet", "暂无重置").to_owned())
                    })
                    .text_size(px(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || theme().foreground),
                ),
        )
        .child(
            text(move || {
                latest(state)
                    .and_then(|reset| format::parse(&reset.announced_at))
                    .map(format::absolute)
                    .unwrap_or_default()
            })
            .text_size(px(12.0))
            .text_color(move || theme().muted_foreground),
        )
        .child(view().pt(px(8.0)).child(separator()))
}

fn next_card(state: State) -> impl IntoElement {
    when(
        move || scheduled_at(state).is_some(),
        move || announced_next(state),
    )
}

fn announced_next(state: State) -> impl IntoElement {
    let foreground = AdaptiveColor {
        light: rgb(0x92400e).into(),
        dark: rgb(0xffb35c).into(),
    };
    button()
        .flex_col()
        .gap(px(5.0))
        .px(px(10.0))
        .py(px(8.0))
        .rounded(px(7.0))
        .bg(|| AdaptiveColor {
            light: rgb(0xffedcc).into(),
            dark: rgb(0x452508).into(),
        })
        .accessibility_label(move || Some(tr("Open scheduled reset announcement", "打开预定重置公告").into()))
        .hover(|style| style.bg(AdaptiveColor {
            light: rgb(0xffdfaa).into(),
            dark: rgb(0x59320d).into(),
        }))
        .on_press(move || {
            if let Some(url) = state
                .status
                .get()
                .and_then(|data| data.scheduled_reset)
                .and_then(|reset| reset.url().map(str::to_owned))
            {
                open(url);
            }
        })
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || tr("Next reset", "下次重置"))
                        .text_size(px(13.0))
                        .text_color(move || foreground)
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(view().flex_1())
                .child(
                    text(move || {
                        scheduled_at(state)
                            .map(|at| format::countdown(at, state.now.get()))
                            .unwrap_or_default()
                    })
                    .text_size(px(11.0))
                    .text_color(move || foreground),
                ),
        )
        .child(
            text(move || {
                scheduled_at(state)
                    .map(|at| format!("{} · {} ↗", format::absolute(at), tr("Scheduled", "已预定")))
                    .unwrap_or_default()
            })
            .text_size(px(11.0))
            .text_color(move || foreground),
        )
}

fn watch_row(state: State) -> impl IntoElement {
    when(
        move || {
            state
                .status
                .get()
                .and_then(|data| data.active_watch)
                .is_some()
        },
        move || {
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .px(px(11.0))
                .py(px(7.0))
                .rounded(px(8.0))
                .bg(move || tint(warning(), 0.12, 0.18))
                .border(px(1.0))
                .border_color(move || tint(warning(), 0.35, 0.4))
                .child(
                    text(move || {
                        state
                            .status
                            .get()
                            .and_then(|data| data.active_watch)
                            .map(|watch| match watch.reset_chance_percent {
                                Some(chance) => format!("{} · {chance}% {}", tr("Watch window", "观察窗口"), tr("chance", "概率")),
                                None => format!("{} · {}", tr("Watch window", "观察窗口"), watch.level),
                            })
                            .unwrap_or_default()
                    })
                    .text_size(px(11.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(warning),
                )
                .child(view().flex_1())
                .child(
                    text(move || {
                        state
                            .status
                            .get()
                            .and_then(|data| data.active_watch)
                            .and_then(|watch| format::parse(&watch.expires_at))
                            .map(|at| format::relative(at, state.now.get()))
                            .unwrap_or_default()
                    })
                    .text_size(px(11.0))
                    .text_color(move || theme().muted_foreground),
                )
        },
    )
}

fn stats_row(state: State) -> impl IntoElement {
    view()
        .flex_row()
        .gap(px(12.0))
        .py(px(4.0))
        .child(
            view()
                .flex_col()
                .flex_1()
                .gap(px(4.0))
                .child(
                    text(move || tr("Total resets", "重置总次数"))
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || {
                        state
                            .status
                            .get()
                            .map(|data| format!("{}", data.stats.total))
                            .unwrap_or_default()
                    })
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(move || reset_color(false)),
                ),
        )
        .child(
            view()
                .flex_col()
                .flex_1()
                .gap(px(4.0))
                .child(
                    text(move || tr("Avg. interval", "平均间隔"))
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || {
                        state
                            .status
                            .get()
                            .and_then(|data| data.stats.avg_interval_days)
                            .map(|days| format!("{} {}", format::number(days), tr("days", "天")))
                            .unwrap_or_else(|| tr("No data", "暂无数据").to_owned())
                    })
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(blue),
                ),
        )
        .child(
            view()
                .flex_col()
                .flex_1()
                .gap(px(4.0))
                .child(
                    text(move || tr("Longest wait", "最长等待"))
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || {
                        longest_wait_days(&state.history.get(), state.has_more.get())
                            .map(|days| format!("{} {}", format::number(days), tr("days", "天")))
                            .unwrap_or_else(|| {
                                if state.busy.get() { tr("Loading…", "加载中…") } else { tr("No data", "暂无数据") }.to_owned()
                            })
                    })
                    .text_size(px(15.0))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(blue),
                ),
        )
}

/// Color carries reset type; neutral cells preserve the quiet menu surface.
fn blue() -> AdaptiveColor {
    AdaptiveColor {
        light: rgb(0x2563eb).into(),
        dark: rgb(0x7cafff).into(),
    }
}

fn reset_color(banked: bool) -> AdaptiveColor {
    if banked {
        AdaptiveColor {
            light: rgb(0xea861c).into(),
            dark: rgb(0xffb04b).into(),
        }
    } else {
        AdaptiveColor {
            light: rgb(0x0c9b63).into(),
            dark: rgb(0x36d399).into(),
        }
    }
}

fn no_reset_color() -> AdaptiveColor {
    AdaptiveColor {
        light: rgb(0xe5eaf1).into(),
        dark: rgb(0x303845).into(),
    }
}

fn unknown_day_color() -> AdaptiveColor {
    AdaptiveColor {
        light: rgb(0xf3f5f8).into(),
        dark: rgb(0x191d24).into(),
    }
}

fn reset_calendar(state: State) -> impl IntoElement {
    let grid = computed(move || {
        calendar::days(
            &state.history.get(),
            state.has_more.get()
                || (state.history.get().is_empty()
                    && state.status.get().is_some_and(|data| data.stats.total > 0)),
            calendar::today(state.now.get()),
        )
    });
    view()
        .flex_col()
        .gap(px(8.0))
        .pb(px(12.0))
        .child(
            view()
                .flex_row()
                .items_center()
                .child(
                    text(move || tr("Reset activity", "重置记录"))
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(view().flex_1())
                .child(
                    text(move || tr("20 weeks · Local time", "20 周 · 本地时间"))
                        .text_size(px(10.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            view()
                .flex_row()
                .gap(px(3.0))
                .child(view().flex_col().gap(px(3.0)).w(px(18.0)).children(
                    [ ("M", "一"), ("", ""), ("W", "三"), ("", ""), ("F", "五"), ("", ""), ("S", "日") ].into_iter().map(|(en, zh)| {
                        text(move || tr(en, zh))
                            .text_size(px(9.0))
                            .h(px(12.0))
                            .text_color(move || theme().muted_foreground)
                    }),
                ))
                .children((0..calendar::WEEKS).map(move |week| {
                    view()
                        .flex_col()
                        .gap(px(3.0))
                        .children((0..7).map(move |weekday| {
                            let index = week * 7 + weekday;
                            view()
                                .size(px(12.0))
                                .rounded(px(3.0))
                                .bg(move || {
                                    let day = &grid.get()[index];
                                    if day.future {
                                        AdaptiveColor::TRANSPARENT
                                    } else if day.banked > 0 {
                                        reset_color(true)
                                    } else if day.regular > 0 {
                                        blue()
                                    } else if !day.known {
                                        unknown_day_color()
                                    } else {
                                        no_reset_color()
                                    }
                                })
                                .tooltip(
                                    tooltip()
                                        .px(px(8.0))
                                        .py(px(5.0))
                                        .rounded(px(4.0))
                                        .bg(move || theme().popover)
                                        .border(px(1.0))
                                        .border_color(move || theme().border)
                                        .child(
                                            text(move || grid.get()[index].hint())
                                                .text_size(px(11.0))
                                                .text_color(move || theme().popover_foreground),
                                        ),
                                )
                                .on_press(move || {
                                    if let Some(url) = grid.get()[index].url.clone() {
                                        open(url);
                                    }
                                })
                        }))
                })),
        )
        .child(
            view()
                .flex_row()
                .items_center()
                .gap(px(6.0))
                .child(
                    text(move || {
                        let days = grid.get();
                        format!(
                            "{} – {}",
                            days[0].date.format("%m/%d"),
                            calendar::today(state.now.get()).format("%m/%d")
                        )
                    })
                    .text_size(px(9.0))
                    .text_color(move || theme().muted_foreground),
                )
                .child(view().flex_1())
                .child(view().size(px(7.0)).rounded(px(2.0)).bg(blue()))
                .child(
                    text(move || tr("regular", "常规"))
                        .text_size(px(9.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(view().size(px(7.0)).rounded(px(2.0)).bg(reset_color(true)))
                .child(
                    text(move || tr("banked", "储备"))
                        .text_size(px(9.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(view().size(px(7.0)).rounded(px(2.0)).bg(no_reset_color))
                .child(
                    text(move || tr("No resets", "无重置"))
                        .text_size(px(9.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

fn error_banner(state: State) -> impl IntoElement {
    when(
        move || state.error.get().is_some() && state.status.get().is_some(),
        move || view()
            .mb(px(12.0))
            .px(px(10.0)).py(px(8.0)).rounded(px(7.0))
            .bg(|| AdaptiveColor {
                light: rgb(0xfee2e2).into(),
                dark: rgb(0x450a0a).into(),
            })
            .child(text(move || tr(
                "Some data could not be updated. Click refresh at the top to try again.",
                "部分数据暂时无法更新，点击顶部刷新即可重试。",
            )).text_size(px(11.0))
                .text_color(|| AdaptiveColor {
                    light: rgb(0x991b1b).into(),
                    dark: rgb(0xfca5a5).into(),
                }).number_of_lines(2)),
    )
}

fn history_section(state: State) -> impl IntoElement {
    view()
        .flex_col()
        .flex_1()
        .overflow_hidden()
        .bg(|| AdaptiveColor {
            light: rgb(0xf2f5fa).into(),
            dark: rgb(0x151c28).into(),
        })
        .child(separator())
        .child(
            view()
                .flex_row()
                .items_center()
                .px(px(PAD))
                .h(px(34.0))
                .child(
                    text(move || tr("Recent resets", "近期重置"))
                        .text_size(px(11.0))
                        .text_color(blue)
                        .font_weight(FontWeight::SEMIBOLD),
                )
                .child(view().flex_1())
                .child(load_more(state)),
        )
        .child(
            scroll().flex_1().child(
                view()
                    .flex_col()
                    .px(px(6.0))
                    .py(px(4.0))
                    .child(each(
                        state.history,
                        |reset| reset.id.clone(),
                        move |row, index| history_row(state, row, index),
                    ))
                    .child(when(
                        move || state.history.get().is_empty(),
                        move || {
                            text(move || tr("No reset records yet", "暂无重置记录"))
                                .text_size(px(12.0))
                                .text_color(move || theme().muted_foreground)
                                .p(px(8.0))
                        },
                    )),
            ),
        )
}

fn load_more(state: State) -> impl IntoElement {
    when(
        move || state.has_more.get(),
        move || {
            button()
                .px(px(7.0))
                .py(px(2.0))
                .rounded(px(5.0))
                .disabled(move || state.busy.get())
                .hover(|style| {
                    style.bg(AdaptiveColor {
                        light: rgb(0xe2ebff).into(),
                        dark: rgb(0x243a60).into(),
                    })
                })
                .active(|style| style.opacity(0.6))
                .cursor_pointer()
                .on_press(move || {
                    if !state.busy.get() && state.has_more.get() {
                        spawn(load_more_task(state));
                    }
                })
                .child(text(move || tr("Load more", "加载更多")).text_size(px(11.0)).text_color(blue))
        },
    )
}

/// A compact history row; dividers appear only between announcements.
fn history_row(state: State, row: ReadSignal<Reset>, index: ReadSignal<usize>) -> impl IntoElement {
    button()
        .flex_col()
        .bg(move || if unread_record(state, &row.get()) { unread_background() } else { AdaptiveColor::TRANSPARENT })
        .rounded(move || px(if unread_record(state, &row.get()) { 7.0 } else { 0.0 }))
        .min_h(px(64.0))
        .flex_none()
        .px(px(8.0))
        .py(px(7.0))
        .gap(px(4.0))
        .border_t(move || px(if index.get() == 0 { 0.0 } else { 1.0 }))
        .border_color(move || tint(blue(), 0.12, 0.18))
        .hover(move |style| {
            style.bg(move || if unread_record(state, &row.get()) { tint(unread_foreground(), 0.2, 0.25) } else { AdaptiveColor {
                light: rgb(0xe2ebff).into(),
                dark: rgb(0x243a60).into(),
            } })
        })
        .on_press(move || {
            if let Some(url) = row.get().url().map(str::to_owned) {
                open(url);
            }
        })
        .child(
            view()
                .flex_row()
                .items_center()
                .gap_2()
                .child(
                    text(move || {
                        format::parse(&row.get().announced_at)
                            .map(|at| {
                                format!("{}  {}", format::short_date(at), format::clock_time(at))
                            })
                            .unwrap_or_default()
                    })
                    .text_size(px(11.0))
                    .text_color(move || if unread_record(state, &row.get()) { unread_foreground() } else { theme().muted_foreground }),
                )
                .child(view().flex_1())
                .child(
                    text(move || {
                        let reset = row.get();
                        if reset.reset_type == ResetType::Banked {
                            reset.reset_type.label()
                        } else if reset.source.as_ref().is_some_and(|s| s.kind == "observed") {
                            tr("Observed", "已观测")
                        } else {
                            tr("Announcement", "公告")
                        }
                    })
                    .text_size(px(10.0))
                    .text_color(move || if unread_record(state, &row.get()) { unread_foreground() } else { theme().muted_foreground }),
                ),
        )
        .child(
            view()
                .w_full()
                .tooltip(
                    tooltip()
                        .w(px(300.0))
                        .p(px(10.0))
                        .rounded(px(6.0))
                        .bg(move || theme().popover)
                        .border(px(1.0))
                        .border_color(move || theme().border)
                        .child(
                            text(move || row.get().text.trim().to_owned())
                                .text_size(px(12.0))
                                .text_color(move || theme().popover_foreground),
                        ),
                )
                .child(
                    text(move || row.get().text.trim().to_owned())
                    .text_size(px(12.0))
                    .text_color(move || if unread_record(state, &row.get()) { unread_foreground() } else { theme().foreground })
                    .number_of_lines(3),
                ),
        )
}

fn attribution_footer() -> impl IntoElement {
    view()
        .flex_col()
        .flex_none()
        .px(px(6.0))
        .pb(px(BOTTOM))
        .child(view().px(px(8.0)).pb(px(4.0)).child(separator()))
        .child(
            view()
                .flex_row()
                .items_center()
                .justify_center()
                .gap(px(3.0))
                .pt(px(6.0))
                .child(
                    text(move || tr("Data from", "数据来源"))
                        .text_size(px(10.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    button()
                        .accessibility_label(move || Some(tr("Open Codex Resets website", "打开 Codex Resets 网站").into()))
                        .hover(|style| style.opacity(0.75))
                        .on_press(|| open(api::SITE))
                        .child(text("Codex Resets").text_size(px(10.0)).text_color(blue)),
                ),
        )
}

fn separator() -> impl IntoElement {
    view().h(px(1.0)).w_full().bg(move || theme().border)
}

/// `CODEX_RESET_TRACE=1` writes what the panel does to
/// `/tmp/codex-reset-trace.log` (a development aid: the panel has no console).
fn trace(message: &str) {
    if std::env::var_os("CODEX_RESET_TRACE").is_none() {
        return;
    }
    use std::io::Write;
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open("/tmp/codex-reset-trace.log")
    {
        let _ = writeln!(file, "{}", message);
    }
}

/// The watch window's hint (amber): a reset may come, no announcement yet.
fn warning() -> AdaptiveColor {
    AdaptiveColor {
        light: rgb(0xb45309).into(),
        dark: rgb(0xfbbf24).into(),
    }
}

/// A theme color at lower opacity, for tints and hovers.
fn tint(color: AdaptiveColor, light: f32, dark: f32) -> AdaptiveColor {
    AdaptiveColor {
        light: color.light.alpha(light),
        dark: color.dark.alpha(dark),
    }
}

// ───────────────────────────── data ─────────────────────────────

fn latest(state: State) -> Option<Reset> {
    state.status.get().and_then(|data| data.latest_reset)
}

fn scheduled_at(state: State) -> Option<i64> {
    state
        .status
        .get()
        .and_then(|data| data.scheduled_reset)
        .and_then(|reset| reset.scheduled_for.or(Some(reset.announced_at)))
        .and_then(|at| format::parse(&at))
}

fn remember_error(state: State, message: String) {
    trace(&format!("refresh failed: {message}"));
    if state.error.get().is_none() {
        state.error.set(Some(message));
    }
}

async fn reload_all(state: State) {
    if state.busy.get() {
        return;
    }
    let started = std::time::Instant::now();
    trace("reload: start");
    state.busy.set(true);
    state.flash.set(false);
    if state.status.get().is_none() {
        state.loading.set(true);
    }

    let mut refreshed = true;
    match api::status().await {
        Ok(data) => {
            crate::monitor::displayed(&data);
            state.status.set(Some(data));
            state.updated_at.set(Some(format::now_epoch()));
            state.error.set(None);
        }
        Err(message) => {
            remember_error(state, message);
            refreshed = false;
        }
    }

    // Keep the displayed snapshot intact while all pages are fetched. Publishing
    // the first page used to erase older calendar cells on every panel open.
    match fetch_history().await {
        Ok(rows) => {
            if state.history.get() != rows {
                state.history.set(rows);
            }
            if state.has_more.get() {
                state.has_more.set(false);
            }
            if state.cursor.get().is_some() {
                state.cursor.set(None);
            }
        }
        Err(message) => {
            remember_error(state, message);
            refreshed = false;
        }
    }

    state.loading.set(false);
    remember(state);
    if refreshed {
        flash(state);
    }

    // However fast the answer, the button shows that it worked.
    if let Some(left) = MIN_BUSY.checked_sub(started.elapsed()) {
        timer(left).await;
    }
    state.busy.set(false);
    trace("reload: done");
}

/// Fetch a complete replacement off-screen; any failed page leaves the current
/// history (and its pagination metadata) untouched.
async fn fetch_history() -> Result<Vec<Reset>, String> {
    collect_history(|cursor| async move { api::resets(PAGE, cursor.as_deref()).await }).await
}

async fn collect_history<F, Fut>(mut fetch_page: F) -> Result<Vec<Reset>, String>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<ResetPage, String>>,
{
    let mut rows = Vec::new();
    let mut cursor = None;
    loop {
        let page = fetch_page(cursor.clone()).await?;
        rows.extend(page.data);
        if !page.pagination.has_more {
            return Ok(rows);
        }
        let next = page.pagination.next_cursor;
        if next.is_none() || next == cursor {
            return Err("Incomplete history: pagination did not advance".to_owned());
        }
        cursor = next;
    }
}

/// The header says "Updated" for a moment, in the accent color.
fn flash(state: State) {
    state.flash.set(true);
    spawn(async move {
        timer(Duration::from_millis(2200)).await;
        state.flash.set(false);
    });
}

async fn load_more_task(state: State) {
    let Some(cursor) = state.cursor.get() else {
        return;
    };
    state.busy.set(true);

    match api::resets(PAGE, Some(&cursor)).await {
        Ok(page) => {
            let mut rows = state.history.get();
            rows.extend(page.data);
            state.history.set(rows);
            state.has_more.set(page.pagination.has_more);
            state.cursor.set(page.pagination.next_cursor);
            state.error.set(None);
        }
        Err(message) => remember_error(state, message),
    }

    state.busy.set(false);
    remember(state);
}

/// Writes what the panel shows to the cache, so the next launch paints it.
fn remember(state: State) {
    let Some(status) = state.status.get() else {
        return;
    };
    cache::save(&cache::Cache {
        saved_at: state.updated_at.get().unwrap_or_else(format::now_epoch),
        status,
        history: state.history.get(),
        cursor: state.cursor.get(),
        has_more: state.has_more.get(),
    });
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    use std::future::{Future, ready};
    use std::task::{Context, Poll, Waker};

    #[test]
    fn unread_highlights_only_records_between_seen_and_latest_reset() {
        assert!(!in_unread_range(Some(10), Some(10), Some(30)));
        assert!(in_unread_range(Some(10), Some(20), Some(30)));
        assert!(in_unread_range(Some(10), Some(30), Some(30)));
        assert!(!in_unread_range(Some(10), Some(40), Some(30)));
        assert!(!in_unread_range(None, Some(20), Some(30)));
        assert!(!in_unread_range(Some(10), None, Some(30)));
        assert!(!in_unread_range(Some(10), Some(20), None));
        // Reopening after acknowledgment clears the highlight.
        assert!(!in_unread_range(Some(30), Some(30), Some(30)));
    }

    fn page(id: &str, more: bool) -> ResetPage {
        ResetPage {
            data: vec![Reset {
                id: id.into(), reset_type: ResetType::Regular,
                announced_at: "2026-10-26T07:00:00Z".into(),
                text: id.into(), source: None,
            }],
            pagination: Pagination {
                has_more: more, next_cursor: more.then(|| id.into()),
            },
            meta: None,
        }
    }

    fn run(pages: Vec<Result<ResetPage, String>>) -> Result<Vec<Reset>, String> {
        let mut pages = pages.into_iter();
        let mut task = std::pin::pin!(collect_history(|_| ready(pages.next().unwrap())));
        match task.as_mut().poll(&mut Context::from_waker(Waker::noop())) {
            Poll::Ready(result) => result,
            Poll::Pending => panic!("fixture pages are immediately ready"),
        }
    }

    #[test]
    fn refresh_returns_complete_history_or_error_never_first_page_only() {
        let first = page("new", true);
        let last = page("old", false);
        let mut expected = first.data.clone();
        expected.extend(last.data.clone());
        assert_eq!(run(vec![Ok(first.clone()), Ok(last)]).unwrap(), expected);
        assert!(run(vec![Ok(first), Err("offline".into())]).is_err());
    }

    #[test]
    fn incomplete_pagination_cannot_replace_displayed_history() {
        assert!(run(vec![Ok(page("same", true)), Ok(page("same", true))]).is_err());
        let mut missing_cursor = page("new", true);
        missing_cursor.pagination.next_cursor = None;
        assert!(run(vec![Ok(missing_cursor)]).is_err());
    }
}
