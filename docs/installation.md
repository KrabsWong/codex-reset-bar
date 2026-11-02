# Installation and updates

The current distribution is for **Apple Silicon Macs (arm64)** running **macOS 14 or later**.

1. Download the ZIP from the [latest release](https://github.com/KrabsWong/codex-reset-bar/releases/latest) and extract it.
2. Use **Finder** to move `Codex Resets.app` to Applications.
3. Open the app and look for its avatar in the menu bar.

The app does not appear in the Dock. Click the avatar to toggle the popover; press Escape or click outside to dismiss it. Right-click (or Control-click) the avatar to see the app version and automatic refresh interval, or choose **Quit Codex Resets**. For automatic startup, add the app under **System Settings → General → Login Items**.

When upgrading manually, quit the running app before replacing it in Applications. Version 0.1.0 has no Quit menu: open macOS Activity Monitor, find `CodexResets`, and quit that process first. Then replace the app and launch the new version.

Distributed and locally built packages use an ad-hoc signature and are not Developer ID signed or notarized. If first opening is blocked, approve this app under **System Settings → Privacy & Security → Open Anyway**, then complete password or Touch ID confirmation. Opening from the download location can prevent updates; install using Finder first.

Right-click the menu bar icon and choose **Check for Updates…**. Release builds check for updates every 12 hours by default; turn this off under **Settings → Automatically check for updates**. Background checks stay silent: an available update adds a blue upward-arrow badge to the menu bar icon and panel avatar. The right-click action becomes **Update to vX.Y.Z**. Click that action or the panel avatar to open Sparkle; it asks before installation and restart. Dismissing or skipping the update clears the badge for that session. Updates and the update feed are verified with the embedded Ed25519 public key.

Users on older versions without Sparkle must install the first updater-enabled release manually. See [release setup](releasing.md) for the permanent signing key and GitHub Actions configuration.

The update window displays an English release summary before installation, with a
link to the full release notes on GitHub. Summaries use New Features, Bug Fixes,
and Improvements when applicable; a release without a detailed summary shows
“Fixed some known issues.”

[Back to README](../README.md)
