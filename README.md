# Codex Resets Bar

A macOS menu bar app for following [Codex Resets](https://codex-resets.com) announcements without keeping a browser tab open.

Built in Rust with [Native Shell](https://nsh.dev) for the interface and AppKit for the menu bar and popover. This is an independent client; reset data is provided by Codex Resets.

| Light | Dark |
|:-----:|:-----:|
<img width="400" alt="image" src="https://github.com/user-attachments/assets/534c9c1c-8650-4e9f-a60a-11157e410c67" /> | <img width="399" alt="image" src="https://github.com/user-attachments/assets/bb692ab5-5d32-4215-a333-8ccb49fe686b" />



## Features

- **Reset statistics:** latest reset, total resets, average interval, and longest wait.
- **Activity calendar:** 20 weeks of history, grouped by local date. Blue represents regular resets, orange represents banked resets, and gray represents days without reset events.
- **Announcement history:** original messages, up to three lines per row, with the full text available on hover. Click a record to open its source when available.
- **Announced schedules:** upcoming resets appear only when provided by the service. The app does not predict the next reset from historical intervals.
- **Background monitoring:** checks status at launch and approximately every 60 seconds, even when the popover has never been opened.
- **Native appearance:** system light and dark modes, local caching, and manual refresh with a spinning indicator while requests are in progress.
- **Language and avatar:** right-click the menu bar icon and choose **Settings** to open a separate native settings window and switch between English (default) and Simplified Chinese, or upload a custom avatar for the menu bar and popover. Preferences survive restarts; announcement messages remain in their original language.

## Menu bar indicators

The avatar displays a small colored dot when there is something to check:

| Indicator | Meaning |
| --- | --- |
| Green | A new reset has not yet been viewed. Open the popover and let the latest record load to clear it. |
| Orange | A reset has been announced and is still awaiting execution. Opening the popover does not clear it. |
| No dot | No unread reset or pending announcement is known. |

The panel uses orange for pending resets, green for unread history rows, and red for refresh errors. Green history highlights remain during the current viewing session and clear when the panel is reopened.

Green takes priority when both states apply. After viewing the new reset, an outstanding announcement can still show an orange dot.

On a first launch without saved state, the first successful response establishes a baseline rather than flagging an old event as new. Viewed timestamps persist across restarts. Failed requests retain the last known state and are retried on the next polling cycle; indicators are not a guarantee of real-time delivery.

## Install

The current distribution is for **Apple Silicon Macs (arm64)** running **macOS 14 or later**.

1. Extract the distribution ZIP.
2. Use **Finder** to move `Codex Resets.app` to Applications.
3. Open the app and look for its avatar in the menu bar.

The app does not appear in the Dock. Click the avatar to toggle the popover; press Escape or click outside to dismiss it. Right-click (or Control-click) the avatar to see the app version and automatic refresh interval, or choose **Quit Codex Resets**. For automatic startup, add the app under **System Settings → General → Login Items**.

When upgrading manually, quit the running app before replacing it in Applications. Version 0.1.0 has no Quit menu: open macOS Activity Monitor, find `CodexResets`, and quit that process first. Then replace the app and launch the new version.

Local packages use an ad-hoc signature and are not Developer ID signed or notarized. If first opening is blocked, approve this app under **System Settings → Privacy & Security → Open Anyway**, then complete password or Touch ID confirmation. Opening from the download location can prevent updates; install using Finder first.

Right-click the menu bar icon and choose **Check for Updates…**. Release builds check for updates every 12 hours by default; turn this off under **Settings → Automatically check for updates**. Background checks stay silent: an available update adds a blue upward-arrow badge to the menu bar icon and panel avatar. The right-click action becomes **Update to vX.Y.Z…**. Click that action or the panel avatar to open Sparkle; it asks before installation and restart. Dismissing or skipping the update clears the badge for that session. This preference is saved by Sparkle in macOS user defaults, separately from `settings.json`. Updates and the update feed are verified with the embedded Ed25519 public key.

Users on older versions without Sparkle must install the first updater-enabled release manually. See [release setup](docs/releasing.md) for the permanent signing key and GitHub Actions configuration.

## Build from source

Requirements: macOS 14+, Xcode Command Line Tools, and a Rust toolchain supporting edition 2024.

```sh
# Run from source.
cargo run

# Build a release app bundle and open it (public key from release setup).
export SPARKLE_PUBLIC_ED_KEY="<your permanent public key>"
bash scripts/package.sh --run
```

The packaging script downloads a checksum-pinned Sparkle SDK, generates the icons, builds the arm64 release executable, embeds Sparkle in `dist/Codex Resets.app`, and signs nested code and the app ad-hoc. Both version fields come from Cargo; the release workflow synchronizes Cargo to the `vX.Y.Z` tag in its build checkout. Source-only `cargo run` builds keep updates disabled; the packaged build enables them. On an Intel build host, install the `aarch64-apple-darwin` Rust target first.

For a local ZIP only (official update releases must also be signed using the [release workflow](docs/releasing.md)):

```sh
ditto -c -k --sequesterRsrc --keepParent \
  'dist/Codex Resets.app' \
  dist/Codex-Resets-0.1.4-macOS-arm64.zip
```

## Data and storage

The app reads the service's public endpoints:

| Endpoint | Purpose |
| --- | --- |
| `GET /api/v1/status` | Latest reset, announced schedule, watch status, and summary statistics. |
| `GET /api/v1/resets?limit=30&order=desc` | Cursor-paginated announcement history. |

**Longest wait** is calculated from the complete history as the largest interval between consecutive reset timestamps. Partial history or invalid dates do not produce a misleading all-time value. Calendar dates use the Mac's local time zone.

Files are stored in `~/Library/Application Support/CodexResets/`:

- `cache.json`: previously fetched statistics and history, displayed while fresh data loads.
- `seen.json`: the last viewed reset timestamp used by the menu bar indicator.
- `settings.json`: the selected interface language.
- `avatar.png`: a local copy of the custom avatar, removed when restoring the default.

Opening the popover refreshes its detailed data. Existing history stays visible until all pages have loaded; an unchanged result does not replace the list, and a failed page leaves the previous history intact. Background status checks use the system's `NSURLSession` and bypass local HTTP caches; panel requests use `Cache-Control: no-cache`. Network failures preserve available data and show a plain-language message pointing to the refresh button. Technical diagnostics are only logged when `CODEX_RESET_TRACE` is enabled.

## Development and verification

```sh
# Unit and local HTTP integration tests.
cargo test

# Run the isolated, real 60-second background polling test.
cargo test background_poll_without_ui -- --ignored --test-threads=1

# Render the panel for visual inspection.
cargo run -- --snapshot /tmp/panel.png 5
cargo run -- --snapshot /tmp/dark.png 5 --dark

# Check popover placement and text restoration across repeated opens.
cargo run -- --self-test --redraw-test

# Open the popover immediately on launch.
CODEX_RESET_SHOW_ON_LAUNCH=1 cargo run
```

Optional environment variables:

| Variable | Purpose |
| --- | --- |
| `CODEX_RESET_API_BASE` | Override the API origin for a local test server. |
| `CODEX_RESET_CACHE_PATH` | Isolate the panel cache; the viewed timestamp uses a sibling `.seen.json` file. |
| `CODEX_RESET_TRACE=1` | Write refresh traces to `/tmp/codex-reset-trace.log` and menu badge changes to standard error. |

## Project structure

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | AppKit host, menu bar icon, popover placement, and runtime checks. |
| `src/ui.rs` | NSH components, styles, refresh, and pagination. |
| `src/monitor.rs` | Background status polling, unread state, and badge selection. |
| `src/api.rs` / `src/model.rs` | API requests, response types, and longest-wait calculation. |
| `src/calendar.rs` / `src/format.rs` | Activity calendar, date handling, and formatting. |
| `src/cache.rs` | Panel cache persistence. |
| `scripts/package.sh` / `scripts/make-icon.swift` | App packaging and icon generation. |

## Credits and license

Data from [Codex Resets](https://codex-resets.com), which follows [@thsottiaux](https://x.com/thsottiaux) for reset announcements. The app links back to the service from its title and footer.

The website's avatar is used for the app icon, menu bar icon, and popover header. Its source is documented in [resources/README.md](resources/README.md). Third-party data and imagery remain the property of their respective owners.

See [LICENSE](LICENSE) for the source code license.
