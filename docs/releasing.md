# Releasing Codex Resets

The release workflow builds an Apple Silicon app for macOS 14+, signs the app
and embedded code ad-hoc, and publishes a ZIP plus an Ed25519-signed Sparkle
appcast on GitHub Releases. No Apple Developer account, Developer ID certificate,
or notarization credentials are used.

The stable feed is:

```text
https://github.com/KrabsWong/codex-reset-bar/releases/latest/download/appcast.xml
```

Every stable release must include `appcast.xml`. Do not manually mark an older
release or one without that asset as Latest. The repository and release assets
must be publicly downloadable; the app does not embed GitHub credentials.

## One-time signing configuration

Create a **new, permanent** Sparkle signing key for this app. Do not reuse any
of the ephemeral keys from `scripts/sparkle-poc/`. After obtaining the pinned SDK
with `bash scripts/fetch-sparkle.sh`, use Sparkle's key generator:

```sh
build/sparkle/sdk/bin/generate_keys --account codex-reset-bar
```

The private key stays in your macOS login keychain. Save the printed public key
as the repository Actions **variable** `SPARKLE_PUBLIC_ED_KEY` under Settings →
Secrets and variables → Actions → Variables.

Export the key to a private temporary file using `generate_keys --account
codex-reset-bar -x <private-file>`; the file contains the base64 seed. Upload its
contents to the repository Actions **secret** `SPARKLE_PRIVATE_ED_KEY`, then
remove the temporary file. The workflow expects the new 32-byte seed format.
For example, with GitHub CLI already authenticated, `gh secret set
SPARKLE_PRIVATE_ED_KEY --repo KrabsWong/codex-reset-bar < <private-file>` sends
it without putting the secret value in command history. Never paste it into
source files, chat, release notes, or build logs.

Keep an encrypted backup of the private key. The public key is bundled in the
app; the release helper derives the public key from the secret and rejects a
mismatch before signing. Losing the private key can require users to reinstall
manually because there is no Developer ID identity for a fallback key rotation.

Setting these repository values is a required setup step; adding the workflow
file does not create them. Restrict write access to release tags and workflow
files because releases install executable code on users' machines.

## Development on multiple Macs

Prefer keeping the release private key in GitHub Actions and in an encrypted
backup (a password manager's secure file or another encrypted vault). Ordinary
development needs no signing private key. Building an updater-enabled local app
requires only `SPARKLE_PUBLIC_ED_KEY`, which can be copied from the repository
Actions variable. Ad-hoc app signing itself needs no shared Apple certificate.

Do not generate a different release key on each computer. All releases must use
the same key pair already trusted by installed applications. Do not assume the
Sparkle login-keychain item will automatically synchronize between Macs. GitHub
Secrets cannot be downloaded again through the settings page and should not be
your only backup.

If a second Mac genuinely needs to sign releases locally, transfer the existing
exported seed through your encrypted vault, fetch the SDK there, then import it:

```sh
build/sparkle/sdk/bin/generate_keys --account codex-reset-bar -f /secure/path/private-key
build/sparkle/sdk/bin/generate_keys --account codex-reset-bar -p
```

Compare the printed public key with the GitHub variable before signing. If an
unrelated key already exists under this account, resolve that mismatch before
importing; do not overwrite the repository's public key to match a new machine.
Remove the plaintext transfer file after import, retaining the encrypted backup.
Never store private keys in the repository, ordinary shared folders, or chat.

## Publish a version

The release tag is the version source of truth. No manual version bump is
required before tagging:

| Change | Example from 0.1.5 | Tag |
| --- | --- | --- |
| Patch | 0.1.6 | `v0.1.6` |
| Minor | 0.2.0 | `v0.2.0` |
| Major | 1.0.0 | `v1.0.0` |

These are explicit versions, not `patch`/`minor`/`major` trigger tags. Choose a
stable version greater than the latest published release.

1. Review and merge the app, release scripts, and workflow changes into `main`.
   Uncommitted local changes are never included in a tag. Run relevant tests;
   scan the exact outgoing Git history before each push.
2. On the intended main commit, create the version tag. For example, for the
   first updater-enabled 0.1.5 release:

   ```sh
   git switch main
   git pull --ff-only origin main
   git tag -a v0.1.5 -m "Release v0.1.5 with automatic updates"
   # After scanning the exact outgoing tag history:
   git push origin v0.1.5
   ```

3. Open GitHub Actions → **Release macOS app**. No manual GitHub Release creation
   is needed. A version-tag push triggers the workflow; ordinary branch pushes
   do not. Git tags have no owning branch, so the workflow explicitly rejects a
   tag whose commit is not reachable from `origin/main`. A historical commit
   already merged into main remains valid if main advanced after tagging.

The workflow fetches full history and tags, verifies tag/checkout/main ancestry,
and synchronizes `Cargo.toml` and `Cargo.lock` in the disposable CI checkout.
`cargo update --workspace` retains existing locked dependency versions. The
Rust `CARGO_PKG_VERSION`, packaged `CFBundleShortVersionString`/`CFBundleVersion`,
ZIP filename, and appcast version all derive from that version. It does not
create a version-bump commit, modify main, or move the release tag. Local source
builds continue to report the checked-in development version.

Next it runs tests, builds the app ad-hoc, exercises updater preference
persistence, checks arm64/signatures, and signs the ZIP and appcast. Only stable
`vMAJOR.MINOR.PATCH` tags are accepted. The first updater-enabled release must
still be installed manually by users of older versions without Sparkle.

Publishing creates a draft, uploads the ZIP and appcast, and marks it Latest
only after both uploads succeed. Failed uploads leave a resumable draft.
Already published releases are not overwritten, and equal/older versions cannot
replace Latest. Release jobs are serialized.

## Automatic release notes

Each GitHub Release body is generated from Git history and saved locally as
`dist/release-notes.md` during publication. It includes:

- The release tag's full commit ID and a commit link.
- A comparison link to the previous latest published release.
- Every commit in `previous_release..current_tag`, including merged commits.
- Each commit's subject (the first line of its commit message) and full linked ID.

This is an exact commit summary, not an AI-generated paraphrase. Commit subjects
are escaped so they cannot inject Markdown/HTML or mention users. The first
published release, when no previous release exists, lists all reachable commits.
An existing previous release must be an ancestor of the new tag; divergent
release histories stop publication rather than silently omit changes. Retrying
a draft regenerates its notes from the same selected history. Keep old release
tags available so the comparison remains reproducible.

## Local asset preparation

To build and sign without publishing, set `SPARKLE_PUBLIC_ED_KEY`, build with
`bash scripts/package.sh`, and provide `SPARKLE_PRIVATE_ED_KEY` securely in the
process environment before running:

```sh
python3 scripts/release.py --tag v0.2.0
```

Replace the example tag with the desired version. For local preparation at an
existing main tag, first run `python3 scripts/prepare_release.py --tag v0.2.0`
before packaging; this modifies the local Cargo files, so use a disposable clean
checkout. Otherwise set the local Cargo version and lockfile to match manually. This creates
`dist/Codex-Resets-<version>-macOS-arm64.zip` and `dist/appcast.xml` without
calling GitHub. The helper sends the private seed to the signer over stdin;
it does not write it to disk or pass it in command-line arguments.

The signed appcast contains the current version only. If a future release raises
the minimum macOS version, design a feed retaining compatible older releases
before publishing it; this simple feed does not maintain a historical fallback.
Delta updates and prerelease channels are not configured.

The bridge smoke check can also be run locally after fetching the SDK:

```sh
python3 scripts/test-updater.py --sdk build/sparkle/sdk
```

It uses a temporary bundle and isolated preference domain; it checks controller
startup and that automatic-check settings survive two relaunches. It does not
replace the browser/Gatekeeper A -> B distribution test.

## Installation and Gatekeeper

Users should extract the ZIP and **use Finder to move the app into Applications**
before opening it. First launch of an ad-hoc signed download may be blocked;
users can approve this app in System Settings → Privacy & Security → Open
Anyway and complete password/Touch ID confirmation. Do not instruct users to
disable Gatekeeper globally or remove quarantine as the default installation
procedure.

Our local browser-download test passed after that approval: Sparkle installed
B over A and restarted it, and B reopened normally. A shell-copy installation
triggered App Translocation and Sparkle error 1005, so copying with a shell is
not equivalent to testing Finder installation. These results do not confer
Apple signing trust or guarantee all macOS versions behave identically.

## References

- [Sparkle signing and setup](https://sparkle-project.org/documentation/)
- [Sparkle publishing](https://sparkle-project.org/documentation/publishing/)
- [GitHub release creation](https://cli.github.com/manual/gh_release_create)
- [GitHub release publication](https://cli.github.com/manual/gh_release_edit)
- [GitHub runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
