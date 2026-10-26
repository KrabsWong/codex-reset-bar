//! The panel's UI: one component, running once, whose texts and rows bind to
//! signals the tasks write.
//!
//! Layout (360 × 728): header, the latest reset, announced upcoming resets, compact statistics, the announcement history, and the API's
//! attribution.

use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use base64::Engine;
use nsh_base::{button, theme, tooltip};
use nsh_core::ui::prelude::*;

use crate::{api, cache, calendar, format, model::*};

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
static SHOW_FRAME: OnceLock<async_channel::Sender<()>> = OnceLock::new();

static OPEN: OnceLock<fn(&str)> = OnceLock::new();
static CLOSE: OnceLock<fn()> = OnceLock::new();

/// Tells the panel it is on screen (the host calls this as the popover
/// opens and closes). The lightweight background monitor remains active.
pub fn set_visible(visible: bool) {
    VISIBLE.store(visible, Ordering::Relaxed);
    crate::monitor::set_visible(visible);
    if visible {
        if let Some(sender) = SHOW_FRAME.get() {
            let _ = sender.try_send(());
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
}

#[component]
pub fn App() -> impl IntoElement {
    // NSH defers text drawing while detached. A visible popover must request
    // a frame even when cached data has not changed, so deferred draws run.
    let (sender, receiver) = async_channel::bounded(1);
    if VISIBLE.load(Ordering::Relaxed) {
        let _ = sender.try_send(());
    }
    let _ = SHOW_FRAME.set(sender);
    // What was read last time paints first: no empty panel, no waiting.
    let cached = cache::load();
    let now = format::now_epoch();
    let state = State {
        status: signal(cached.as_ref().map(|cache| cache.status.clone())),
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
    };

    spawn(async move {
        while receiver.recv().await.is_ok() {
            request_animation_frame(|_| {});
            // A hidden background check may be newer than the displayed cache.
            // Opening the panel refreshes its full history immediately.
            reload_all(state).await;
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
            button()
                .size(px(32.0))
                .accessibility_label("Open Codex Resets website")
                .on_press(|| open(api::SITE))
                .hover(|style| style.opacity(0.75))
                .child(
                    image(format!("data:image/jpeg;base64,{}",
                        base64::engine::general_purpose::STANDARD.encode(
                            include_bytes!("../resources/codex-resets-logo.jpg"))))
                        .size(px(32.0))
                        .rounded(px(16.0)),
                ),
        )
        .child(
            view()
                .flex_col()
                .flex_1()
                .gap(px(3.0))
                .child(
                    button()
                        .accessibility_label("Open Codex Resets website")
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
                .accessibility_label("Refresh")
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
                        .tooltip(tooltip().child(text("Refresh")))
                        .child(text("↻").text_size(px(22.0)).text_color(blue)),
                ),
        )
}

fn subtitle(state: State) -> String {
    if state.busy.get() {
        return "Refreshing…".to_owned();
    }
    if state.flash.get() {
        return match state.updated_at.get() {
            Some(at) => format!("Updated · {}", format::clock_time(at)),
            None => "Updated".to_owned(),
        };
    }
    if state.status.get().is_none() {
        return if state.error.get().is_some() {
            "Unable to connect"
        } else {
            "Connecting…"
        }
        .to_owned();
    }
    match state.updated_at.get() {
        Some(at) => format!("Updated at {}", format::clock_time(at)),
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
    text("We watch ")
        .text_size(px(11.0))
        .text_color(move || theme().muted_foreground)
        .child(
            text("@thsottiaux")
                .text_color(blue)
                .cursor_pointer()
                .on_press(|| open("https://x.com/thsottiaux")),
        )
        .child(text(" for Codex reset announcements, so you don't have to."))
}

fn placeholder(state: State) -> impl IntoElement {
    when(move || state.loading.get(), move || skeleton(state))
        .otherwise(move || failure_view(state))
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

fn failure_view(state: State) -> impl IntoElement {
    view()
        .flex_col()
        .items_center()
        .justify_center()
        .gap_3()
        .h(px(260.0))
        .child(
            text(move || {
                state
                    .error
                    .get()
                    .unwrap_or_else(|| "Unable to load data".to_owned())
            })
            .text_size(px(12.0))
            .text_color(move || theme().destructive)
            .number_of_lines(4),
        )
        .child(
            view()
                .px_3()
                .py(px(6.0))
                .rounded(px(7.0))
                .bg(move || theme().primary)
                .active(|style| style.opacity(0.7))
                .cursor_pointer()
                .on_press(move || {
                    if !state.busy.get() {
                        spawn(reload_all(state));
                    }
                })
                .child(
                    text("Retry")
                        .text_size(px(11.0))
                        .text_color(move || theme().primary_foreground),
                ),
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
                    text("Latest reset")
                        .text_size(px(14.0))
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(view().flex_1())
                .child(
                    text(move || {
                        latest(state)
                            .and_then(|reset| format::parse(&reset.announced_at))
                            .map(|at| format::relative(at, state.now.get()))
                            .unwrap_or_else(|| "No resets yet".to_owned())
                    })
                    .text_size(px(14.0))
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(move || reset_color(false)),
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
    button()
        .flex_col()
        .gap(px(5.0))
        .rounded(px(4.0))
        .accessibility_label("Open scheduled reset announcement")
        .hover(|style| style.bg(theme().accent))
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
                    text("Next reset")
                        .text_size(px(13.0))
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
                    .text_color(blue),
                ),
        )
        .child(
            text(move || {
                scheduled_at(state)
                    .map(|at| format!("{} · Scheduled ↗", format::absolute(at)))
                    .unwrap_or_default()
            })
            .text_size(px(11.0))
            .text_color(blue),
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
                                Some(chance) => format!("Watch window · {chance}% chance"),
                                None => format!("Watch window · {}", watch.level),
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
                    text("Total resets")
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
                    text("Avg. interval")
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || {
                        state
                            .status
                            .get()
                            .and_then(|data| data.stats.avg_interval_days)
                            .map(|days| format!("{} days", format::number(days)))
                            .unwrap_or_else(|| "No data".to_owned())
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
                    text("Longest wait")
                        .text_size(px(11.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    text(move || {
                        longest_wait_days(&state.history.get(), state.has_more.get())
                            .map(|days| format!("{} days", format::number(days)))
                            .unwrap_or_else(|| {
                                if state.busy.get() { "Loading…" } else { "No data" }.to_owned()
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
                    text("Reset activity")
                        .text_size(px(12.0))
                        .font_weight(FontWeight::MEDIUM),
                )
                .child(view().flex_1())
                .child(
                    text("20 weeks · Local time")
                        .text_size(px(10.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
        .child(
            view()
                .flex_row()
                .gap(px(3.0))
                .child(view().flex_col().gap(px(3.0)).w(px(18.0)).children(
                    ["M", "", "W", "", "F", "", "S"].into_iter().map(|label| {
                        text(label)
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
                    text("regular")
                        .text_size(px(9.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(view().size(px(7.0)).rounded(px(2.0)).bg(reset_color(true)))
                .child(
                    text("banked")
                        .text_size(px(9.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(view().size(px(7.0)).rounded(px(2.0)).bg(no_reset_color))
                .child(
                    text("No resets")
                        .text_size(px(9.0))
                        .text_color(move || theme().muted_foreground),
                ),
        )
}

fn error_banner(state: State) -> impl IntoElement {
    when(
        move || state.error.get().is_some() && state.status.get().is_some(),
        move || {
            view()
                .px(px(11.0))
                .py(px(7.0))
                .rounded(px(8.0))
                .bg(move || tint(theme().destructive, 0.10, 0.16))
                .child(
                    text(move || state.error.get().unwrap_or_default())
                        .text_size(px(11.0))
                        .text_color(move || theme().destructive)
                        .number_of_lines(2),
                )
        },
    )
}

// ──────────────────────────── history ────────────────────────────

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
                    text("Recent resets")
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
                        move |row, index| history_row(row, index),
                    ))
                    .child(when(
                        move || state.history.get().is_empty(),
                        move || {
                            text("No reset records yet")
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
                .child(text("Load more").text_size(px(11.0)).text_color(blue))
        },
    )
}

/// A compact history row; dividers appear only between announcements.
fn history_row(row: ReadSignal<Reset>, index: ReadSignal<usize>) -> impl IntoElement {
    button()
        .flex_col()
        .min_h(px(64.0))
        .flex_none()
        .px(px(8.0))
        .py(px(7.0))
        .gap(px(4.0))
        .border_t(move || px(if index.get() == 0 { 0.0 } else { 1.0 }))
        .border_color(move || tint(blue(), 0.12, 0.18))
        .hover(|style| {
            style.bg(AdaptiveColor {
                light: rgb(0xe2ebff).into(),
                dark: rgb(0x243a60).into(),
            })
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
                    .text_color(move || theme().muted_foreground),
                )
                .child(view().flex_1())
                .child(
                    text(move || {
                        let reset = row.get();
                        if reset.reset_type == ResetType::Banked {
                            reset.reset_type.label()
                        } else if reset.source.as_ref().is_some_and(|s| s.kind == "observed") {
                            "Observed"
                        } else {
                            "Announcement"
                        }
                    })
                    .text_size(px(10.0))
                    .text_color(move || theme().muted_foreground),
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
                    text("Data from")
                        .text_size(px(10.0))
                        .text_color(move || theme().muted_foreground),
                )
                .child(
                    button()
                        .accessibility_label("Open Codex Resets website")
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

    match api::resets(PAGE, None).await {
        Ok(page) => {
            state.history.set(page.data);
            state.has_more.set(page.pagination.has_more);
            state.cursor.set(page.pagination.next_cursor);
            state.error.set(None);
            // Longest wait is an all-time statistic: load every page before
            // publishing it, rather than using only the calendar's date range.
            while state.has_more.get() {
                let Some(cursor) = state.cursor.get() else {
                    break;
                };
                match api::resets(PAGE, Some(&cursor)).await {
                    Ok(page) => {
                        let mut rows = state.history.get();
                        rows.extend(page.data);
                        state.history.set(rows);
                        state.has_more.set(page.pagination.has_more);
                        state.cursor.set(page.pagination.next_cursor);
                        if state.cursor.get().as_deref() == Some(cursor.as_str()) {
                            break;
                        }
                    }
                    Err(message) => {
                        remember_error(state, message);
                        refreshed = false;
                        break;
                    }
                }
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
