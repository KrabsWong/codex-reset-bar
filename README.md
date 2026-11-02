# Codex Resets Bar

A macOS menu bar app for following [Codex Resets](https://codex-resets.com) announcements without keeping a browser tab open.

An independent client for **Apple Silicon Macs running macOS 14 or later**. Reset data is provided by Codex Resets.

| Light | Dark |
|:-----:|:-----:|
<img width="400" alt="image" src="https://github.com/user-attachments/assets/534c9c1c-8650-4e9f-a60a-11157e410c67" /> | <img width="399" alt="image" src="https://github.com/user-attachments/assets/bb692ab5-5d32-4215-a333-8ccb49fe686b" />

## Features

- **Reset statistics:** see the latest reset, total resets, average interval, and longest wait.
- **Activity calendar:** browse 20 weeks of history in your local time zone. Blue marks regular resets, orange marks banked resets, and gray marks days without reset events.
- **Announcement history:** read original messages, hover for the full text, and open the source of a record when available.
- **Announced schedules:** see upcoming resets when the service announces them. The app does not predict reset times from historical intervals.
- **Background monitoring:** checks for reset updates at launch and approximately every 60 seconds, even when the panel is closed. Cached data remains available while refreshing or when a request fails.
- **Native menu bar interface:** a compact popover with system light and dark modes, manual refresh, and no Dock icon.
- **Personalization:** choose English or Simplified Chinese and upload a custom avatar for the menu bar and panel. Preferences persist across restarts; announcement messages retain their original language.
- **In-app updates:** quiet background checks, a visible update badge, and release notes inside the update window so you can review changes before installing.

## Reset indicators

The menu bar avatar shows the current reset status:

| Indicator | Meaning |
| --- | --- |
| Green dot | A new reset has not yet been viewed. Open the panel and let the latest record load to clear it. |
| Orange dot | A reset has been announced and is awaiting execution. Viewing the panel does not clear it. |
| No dot | No unread reset or pending announcement is known. |

Green takes priority when both states apply. After viewing a new reset, a pending announcement can still show an orange dot. The panel highlights unread history in green, pending resets in orange, and refresh errors in red.

The first successful check establishes a baseline without marking old events as new. Viewed resets stay remembered across restarts.

## Settings and updates

Right-click the menu bar avatar and choose **Settings** to change the interface language, customize the avatar, or toggle automatic app-update checks.

Release builds check for app updates every 12 hours by default. An available update adds a blue upward-arrow badge to the menu bar and panel avatars. Choose **Update to vX.Y.Z** from the menu or click the panel avatar to view the update. You can also choose **Check for Updates…** at any time.

The update window shows an English summary of changes and a link to the full release notes on GitHub. Installation and restart require your confirmation. Update archives and the update feed are verified with the app's embedded signing key.

## Documentation

- [Download the latest release](https://github.com/KrabsWong/codex-reset-bar/releases/latest)
- [Installation, first launch, and updates](docs/installation.md)
- [Local development, builds, and testing](docs/development.md)
- [Data sources and local storage](docs/data-and-storage.md)
- [Release automation and maintainer setup](docs/releasing.md)
- [Credits and license](docs/credits.md)
