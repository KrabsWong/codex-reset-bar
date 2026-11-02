# Local development

Built in Rust with [Native Shell](https://nsh.dev) for the interface and AppKit
for the menu bar and popover. Run the commands below from the repository root.

## Build from source

Requirements: macOS 14+, Xcode Command Line Tools, and a Rust toolchain supporting edition 2024.

```sh
# Run from source.
cargo run

# Build a release app bundle and open it (public key from release setup).
export SPARKLE_PUBLIC_ED_KEY="<your permanent public key>"
bash scripts/package.sh --run
```

The packaging script downloads a checksum-pinned Sparkle SDK, generates the icons, builds the arm64 release executable, embeds Sparkle in `dist/Codex Resets.app`, and signs nested code and the app ad-hoc. Both bundle version fields come from Cargo. The automatic release flow updates the package version on main through a version-only PR before creating the formal tag; the build also validates and synchronizes its checkout against that tag. Source-only `cargo run` builds keep updates disabled; the packaged build enables them. On an Intel build host, install the `aarch64-apple-darwin` Rust target first.

For a local ZIP only (official update releases must also be signed using the [release workflow](releasing.md)):

```sh
ditto -c -k --sequesterRsrc --keepParent \
  'dist/Codex Resets.app' \
  dist/Codex-Resets-local-macOS-arm64.zip
```

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

Release automation and Markdown rendering checks run separately:

```sh
python3 -m unittest discover -s scripts -p 'test_*.py'
```

See [release automation](releasing.md) for publishing and signing, and the
[isolated Sparkle experiment](../scripts/sparkle-poc/README.md) for updater
integration tests.

## Project structure

| Path | Responsibility |
| --- | --- |
| `src/main.rs` | AppKit host, menu bar icon, popover placement, and runtime checks. |
| `src/ui.rs` | NSH components, styles, refresh, and pagination. |
| `src/monitor.rs` | Background status polling, unread state, and badge selection. |
| `src/api.rs` / `src/model.rs` | API requests, response types, and longest-wait calculation. |
| `src/calendar.rs` / `src/format.rs` | Activity calendar, date handling, and formatting. |
| `src/cache.rs` | Panel cache persistence. |
| `src/settings.rs` / `src/preferences.rs` / `src/i18n.rs` | Saved preferences, the settings window, and interface translations. |
| `src/updater.rs` | Production updater bridge and available-update state. |
| `scripts/package.sh` / `scripts/make-icon.swift` | App packaging and icon generation. |

[Back to README](../README.md)
