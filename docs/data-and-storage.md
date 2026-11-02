# Data and storage

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

Sparkle stores its automatic-update-check preference in macOS user defaults,
separately from `settings.json`.

[Back to README](../README.md)
