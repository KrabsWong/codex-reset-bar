# Sparkle ad-hoc A/B experiment

This opt-in experiment runs the actual Rust/AppKit app with Sparkle 2.9.3.
It is not the production updater UI. `cargo build` omits Sparkle;
`scripts/package.sh` now enables the separate production `sparkle` feature.
Never enable `sparkle-poc` for distribution:
its test user driver automatically accepts updates and restarts.

## Run

Prerequisites: macOS 14+, Apple Silicon, Rust, Xcode command line tools,
Python 3, and an extracted Sparkle distribution containing `bin/` and
`Sparkle.xcframework/`. The tested SDK came from the existing SwiftPM artifact
cache for Sparkle 2.9.3; no Apple account or signing identity is used.

From the repository root:

```sh
python3 scripts/sparkle-poc/run.py --sdk /absolute/path/to/extracted/Sparkle
python3 scripts/sparkle-poc/run.py --sdk /absolute/path/to/extracted/Sparkle --bad-signature
```

The runner needs permission to bind a loopback HTTP port and launch a GUI app.
It builds in `target/sparkle-poc`, stages bundles in a unique directory under
`build/sparkle-poc`, and launches A through Launch Services. The Objective-C
bridge starts Sparkle after the actual menu bar and background monitor have
been initialized. `app-ready` is written from the main dispatch queue once
AppKit's event loop runs. A and B use the same executable with distinct bundle
versions (`0.0.1`/build `1` and `0.0.2`/build `2`).

Each run uses a unique bundle identifier, isolated application settings/cache,
and an ephemeral Ed25519 seed file. It never accesses the signing keychain.
All nested framework/helper code and the app are re-signed ad-hoc. The seed is
deleted after signing (and in cleanup); it is not a release signing key.
The test driver is guarded by the PoC bundle ID and loopback feed address.

The runner retains app bundles, ZIP, feed, signing metadata, HTTP log, event log,
and `result.json` for inspection. It shuts down its server and stops only its
own test application. Sparkle/macOS may retain caches and preferences under the
unique test bundle ID. The normal installed application is not replaced.

## Results on 2026-10-30

Environment: Apple Silicon, macOS 27.2 (26B123), Sparkle 2.9.3.

- Valid signature: PASS. A (PID 31231) checked the feed, downloaded B, reached
  install/relaunch, and B (PID 31269) entered the real app's main event loop.
  The installed bundle version became `2`; deep/strict code signature
  verification passed. Settings bytes were preserved.
- Invalid signature: PASS. Changing one character of the Ed25519 signature
  caused `SUSparkleErrorDomain` 4005 / underlying 3002 (signature mismatch).
  A remained installed; B was not launched. The error explicitly reported that
  Apple Developer ID fallback was unavailable because A had no team identifier.
- Both bundles: `Signature=adhoc`, `TeamIdentifier=not set`.
- Existing tests: `cargo test --quiet`: 21 passed, 1 ignored.

First successful evidence directories (ignored build artifacts):
`build/sparkle-poc/run-gzv1a02v` and `build/sparkle-poc/run-ury3o963`.

## What this proves, and what it does not

Sparkle can update this app from A to B and relaunch it using ad-hoc code signing
plus an independently generated Ed25519 update signature, without Developer ID.

This test uses localhost HTTP, a user-writable install directory, and locally
created bundles without browser quarantine. It does **not** verify first-download
Gatekeeper approval, upgrading a quarantined browser download, `/Applications`
permissions, other macOS versions, or production UI. No Gatekeeper settings
were disabled and no quarantine attributes were removed. A public release needs
HTTPS, a durable signing key, standard updater UI, and distribution testing.

References:
- https://sparkle-project.org/documentation/
- https://sparkle-project.org/documentation/programmatic-setup/
- https://sparkle-project.org/documentation/publishing/

## Browser / Gatekeeper experiment

```sh
python3 scripts/sparkle-poc/run.py --sdk /absolute/path/to/extracted/Sparkle --distribution
```

This mode prints a loopback browser download URL and a unique `/Applications`
installation target. It serves A.zip as well as the signed B update, waits up to
one hour for B to start, and retains evidence. Download A through the browser,
extract with Archive Utility, **move the extracted app into Applications using
Finder**, then open that installed copy. Handle macOS's per-app approval in
Privacy & Security if offered. Do not remove quarantine or disable Gatekeeper.
This is still a local HTTP origin, not a public HTTPS hosting/reachability test.

2026-10-30 distribution run: `build/sparkle-poc/run-nj1dwae0`.

- Ego Lite download produced a genuine `com.apple.quarantine` attribute:
  `0081;6ae42b9d;ego lite;`. Archive Utility propagated quarantine to the app.
- Gatekeeper was enabled. The initial quarantined application installed by
  `ditto` was rejected by `spctl`; syspolicyd logged a prompt and an open denial.
- A subsequently started at a verified `AppTranslocation/...` runtime path.
  Sparkle rejected updating with error 1005 (running from a download location).
  The agent could not observe that first intervening UI interaction. During
  the subsequent Finder installation the user explicitly confirmed clicking
  Open Anyway and completing password or Touch ID authentication.
- Copying via shell is therefore not sufficient evidence of the normal Finder
  installation flow. The shell-installed test copy was moved back to
  `command-copy-A.app` in the evidence directory and its process was stopped.
- Finder scripting was denied by macOS automation permission; System Events
  accessibility and screen capture were also unavailable. The user manually
  moved the browser-extracted A into Applications and approved first opening.
- After Finder installation and user approval: PASS. A (PID 38084) downloaded
  and installed B; B started as PID 38144 from `/Applications`, not an
  AppTranslocation path. A further normal quit/open started B as PID 39180.
  Both launches reached the main event loop without further blocking.
- B passed deep/strict codesign verification and remained ad-hoc with no
  TeamIdentifier. Its installed bundle had no quarantine attribute after
  Sparkle installation; the test scripts did not manually remove quarantine.
- `spctl --assess` still reported `rejected` for B. Successful normal launch
  does not mean the app acquired Developer ID trust or notarization.
- Settings bytes were preserved. The app was owned by the current user in
  `/Applications`; this does not prove updates to an admin/root-owned install.

No Gatekeeper settings were changed and no quarantine attributes were removed.
`quarantine-before.json` and `gatekeeper.log` record the first attempt.
`gatekeeper-after.log`, `events.log`, and `distribution-result.json` record the
successful Finder installation, update, and subsequent relaunch. The test app
was stopped and moved out of Applications into `verified-B.app` in the evidence
directory after verification; the loopback server was stopped.

Observed user flow on this machine: download ZIP -> extract -> use Finder to
move into Applications -> attempt opening -> Privacy & Security / Open Anyway
-> password or Touch ID -> app starts in menu bar -> Sparkle update and restart.
The test driver auto-accepts the update; a production updater would additionally
show its update/install confirmation UI. No application window is expected from
this menu-bar-only app. Other macOS/browser combinations still need testing.
