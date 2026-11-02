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

The standard flow starts with a `release/<patch|minor|major>/<unique-id>` tag
on latest main. It calculates the next version, automatically merges a validated
version-only PR, creates the formal `vX.Y.Z` tag, and opens a release-notes PR
for human approval and merge. Follow [For each release](#for-each-release).

The two tag forms have different roles:

| Tag | Created by | Role |
| --- | --- | --- |
| `release/patch/<unique-id>`, `release/minor/<unique-id>`, `release/major/<unique-id>` | Maintainer | Requests a release and selects the version increment. |
| `vX.Y.Z` | Release workflow | Records the exact source commit for the calculated version. |

Do not create the formal version tag yourself. Pushing `vX.Y.Z` does not start
release preparation. Existing formal tags remain available for downloads,
source history, and release comparisons; do not move or delete published tags.
Ordinary branch pushes and ordinary code PR merges do not publish the app.

After the release-note PR is approved and merged, the build validates the formal
tag and its main ancestry, synchronizes the build checkout to that version,
runs tests, packages the app, verifies updater preferences, and signs the ZIP
and appcast. It builds the tagged source, not newer changes on main.

Publication creates a draft, uploads the ZIP and appcast, and marks it Latest
only after both uploads succeed. Failed uploads leave a resumable draft.
Already published releases are not overwritten, and equal/older versions cannot
replace Latest. Release jobs are serialized.

## Release-note PR review with OpenCode Go

After a release-request tag is pushed, preparation happens on GitHub: the
workflow updates the version and creates a release-note PR, maintainer
comments request revisions or approve the current draft, and a manual merge
starts packaging and publication. No local OpenCode, notes preparation, or build
is required. Revisions use the official OpenCode GitHub bot (`opencode github run`)
with the existing Actions token and pinned CLI. Installing the OpenCode GitHub App
is optional and is not needed for this configuration.

### One-time account and repository setup

1. Sign in to [OpenCode Console](https://opencode.ai/console) and activate Go in
   the workspace you intend to use.
2. Open **Account → Personal API keys → Add API key**. Select that Go workspace,
   name the key `codex-reset-release`, and copy it when created (shown only once).
   If permissions are offered, choose **Restricted → Models → Request**.
   Read-only keys cannot call models. Use a personal key tied to Go membership;
   service-account billing is separate and may not use that subscription.
3. In GitHub repository **Settings → Secrets and variables → Actions → Secrets**,
   add `OPENCODE_GO_API_KEY`. Never put its value in YAML, source files, or chat.
4. In **Variables**, add `OPENCODE_MODEL`, e.g. `opencode-go/minimax-m3`.
   Choose a model available to your Go workspace. `opencode models opencode-go`
   lists model IDs; the `opencode-go/` prefix is required.
5. Keep the existing `SPARKLE_PUBLIC_ED_KEY` variable and
   `SPARKLE_PRIVATE_ED_KEY` secret. `GITHUB_TOKEN` is supplied by Actions;
   no GitHub PAT or OpenCode GitHub App installation is required.
6. In **Settings → Actions → General → Workflow permissions**, enable
   **Allow GitHub Actions to create and approve pull requests**. Workflow YAML
   grants only each job's necessary permissions; account/organization policy
   must also permit PR creation. The bot does not merge release-note PRs or submit a GitHub
   approval review; it records your explicit comment approval. Version-only PRs
   are validated and merged automatically during version preparation.
7. Merge these workflow changes into `main` before creating the first release
   tag using this process. Comment workflows must exist on the default branch.

References: [Go](https://opencode.ai/v2/docs/console/go/),
[API keys](https://opencode.ai/v2/docs/console/api/#personal-api-keys),
[CLI](https://opencode.ai/docs/cli/).

### For each release

1. Push a release-request tag on the latest main commit. Its format is
   `release/<patch|minor|major>/<unique-id>`, for example:

   ```sh
   git switch main
   git pull --ff-only origin main
   git tag release/patch/20261102-1
   git push origin refs/tags/release/patch/20261102-1
   ```

   Use a fresh identifier for each request; never move or reuse an existing tag.
   Replace `patch` with `minor` or `major` for the other increment levels. The push
   starts Actions automatically: **Run workflow is not required**. A local tag
   alone does not trigger GitHub until pushed. Follow the repository's outgoing
   history secret-scan policy before pushing.

   The request tag must initially point at current main, and its source must be
   in main's ancestry. Requests from feature branches or stale main commits fail.
   Request tags remain as history; they are not application version numbers.
   The workflow checks out trusted main scripts for request processing. It creates
   the final `vX.Y.Z` tag only after the version PR merges. Changes to main during
   preparation stop automatic merging; inspect the version PR before retrying.

   As a fallback, **Actions → Release macOS app → Run workflow** still accepts
   a **level** selection on **main** (`patch` by default). The baseline
   is the latest published release, not the old version in the checkout.
   For `v0.1.10`, these produce `v0.1.11`, `v0.2.0`, and `v1.0.0` respectively.
   With no previous release, the baseline is `v0.0.0`.

   The workflow creates a `release-version/<tag>` PR updating only the root
   package versions in `Cargo.toml` and `Cargo.lock`. It verifies the exact file
   contents and file list, records **Release version validation**, refreshes the
   existing required check, then automatically merges that version-only PR
   through GitHub's normal merge API. It does not bypass branch rules. Additional
   required checks/reviews can block it; inspect the PR and retry after resolving
   them. No dependency versions are upgraded.

   A tag is created at the version PR's merge commit, so main and the tagged
   source both contain the new version. The same run generates the notes PR;
   GitHub-token merges and tags do not trigger another run automatically.
   **Only the version PR is automatically merged; notes still require your
   explicit approval and manual merge.**

   Finish an existing notes PR before starting another release. Different pending
   version PRs or unpublished newer tags also block preparation. Before publication,
   rerunning the original Actions run resumes its managed version PR, including
   after its merge. Tags are never moved. Each run is bound to its version PR by
   the Actions run ID. After publication,
   rerunning that run does not increment again; start a new run for the next version.
   A new run cannot take over another run’s version PR; retry the original run.
   A failure before the version PR is created can leave a preparation branch;
   inspect and remove that unused branch before retrying.

   A manually pushed `vX.Y.Z` tag does not trigger this workflow. Use a
   release-request tag or the Actions level selector to prepare a new version.
2. **Release macOS app → notes-pr** compares the previous latest published
   release with that tag, asks OpenCode to produce the summary, and creates a
   `release-notes/<tag>` branch and PR changing only `release-notes/<tag>.md`.
   Read **Files changed → Rendered view** for the authoritative draft.
3. Request a revision by posting a new PR conversation comment:

   ```text
   /oc Rewrite the first item in user-friendly language and remove internal refactoring details.
   ```

   **Review release notes** validates the request, then its **opencode** job runs
   the official bot, which replies and commits changes to the same PR. `/opencode`
   is also supported. Use a new conversation comment, not an inline review comment.
   Follow progress in **Actions → Review release notes → opencode**; a failed run
   is not a completed revision. Inspect Files changed after completion. You can
   also edit the Markdown file directly in GitHub. The old `/release-notes revise`
   command no longer generates revisions; post a new `/oc` comment instead.
4. Once satisfied, use the current 40-character PR head commit SHA from the
   **Commits** tab (the original PR body's command becomes stale after revisions):

   ```text
   /release-notes approve <full-head-sha>
   ```

   Only repository writers/maintainers/admins can invoke commands. The SHA
   prevents a queued workflow from approving a different draft than you saw.
   If you edited manually, use the latest PR commit SHA from the Commits tab.
   Any subsequent commit or revision request requires fresh approval. Post a
   new comment for each command; editing an old comment does not run the bot.
5. Wait for the bot's **Approved release notes at ...** acknowledgement, then
   merge the PR yourself through GitHub. The bot never merges it for you.
6. The merge triggers **Release macOS app → approved-notes → release**. It
   verifies the original tag has not moved, only the notes file changed, the
   current draft was approved, and merged notes equal the reviewed draft.
   Then the macOS runner tests, packages, signs and publishes automatically.

The app is built from the **original tag commit**, not the PR merge commit or
current main. Notes come from the approved merged PR via the `release-summary`
artifact, retained 30 days. Other main changes during review cannot silently
enter the tagged release. The published GitHub Release contains this same
summary plus complete commit history; Sparkle embeds it before signing.

Protect main with a required commit status named **Release notes approval**, sourced
from GitHub Actions, with no bypass actors. Install the gate workflows on main
before enabling this rule. Run **Refresh release-note approval → Run workflow**
with each existing PR number (including the current release-note PR) to bootstrap
its status. Keep "Require branches to be up to date" off: release-note PRs retain
their original source tag and must not acquire unrelated changes during review.

The status is pending until the latest authorized approval and bot acknowledgement
match the current head SHA. Revision requests, new commits, or edited/deleted
approval comments revoke it. Ordinary code PRs pass automatically. The gate runs
trusted main code and also detects versioned notes files when a PR is renamed.
Bot-created PRs receive their initial pending status directly, since GITHUB_TOKEN
events do not trigger another workflow. Comment processing refreshes the status
before running OpenCode and again afterward, including on failure. The final
check runs on a separate runner with a fresh main checkout; the agent cannot
modify the code used by that check. This explicit refresh is required because
commits made with GITHUB_TOKEN do not trigger another workflow.

Comment-triggered status updates are asynchronous: a brief queueing window remains.
The existing post-merge guard independently rechecks the latest command and stops
publication without matching approval. Follow the acknowledgement-before-merge sequence.
Do not merge through another workflow's `GITHUB_TOKEN`, since that merge may not
trigger the publication workflow; merge as a user. Closing without merging does
not publish. To replace a closed, unmerged draft for an unpublished version, first
remove its old `release-notes/<tag>` branch, then rerun the original preparation
run (for versions created with the level selector). The level selector is intended to prepare versions, not reset tags.
The workflow generates fresh notes in a new PR with a pending approval status.
It refuses to reuse merged notes, an existing release, or a leftover draft branch.

If the unpublished tag must also include newer main code, explicitly remove that
old tag and its version-preparation branch/PR state only after checking the
release history. This is an exceptional manual recovery, not a level-selector
operation. Normal retries retain the original version PR merge commit.
Never delete/recreate tags for a distributed version.

Retry failed builds with **Re-run failed jobs**. Rerunning the original preparation run reuses an
existing open notes PR and never overwrites edits.

Release one version at a time: if another version becomes latest during review,
publication stops rather than publishing notes based on an outdated comparison.
Create a newer tag/PR against the new latest; do not move an existing tag.

### AI input, fallback and isolation

OpenCode receives commit messages and actual code/test differences between the
previous published tag and target tag. On first release, it compares the target
tree against an empty tree. Lockfile and release-note files are excluded. The
prompt writes all headings and bullet text in English, grouping user-visible changes
under `## New Features`, `## Bug Fixes`, and `## Improvements`. This also applies
to revisions requested in another language. It omits empty sections and prohibits
unsupported claims. Markdown structure and
placeholders are validated; your PR review confirms factual accuracy.

Missing credentials/model, CLI installation failure, provider errors, a
180-second generation timeout, invalid output, or comparison size above 300 KB
produce the initial fallback **Fixed some known issues.** The fallback still
requires your approval and merge. Revision requests revoke previous approval;
contexts above 120 KB are rejected before launching the bot (edit manually in
GitHub for these releases). No silently truncated diff is summarized.
Official-bot revisions can create commits before a later failure: always inspect
the final diff and workflow result before approving.

Initial generation runs in a temporary directory with tools denied and comparison
text as input; it receives no GitHub or signing credentials. Revisions instead
use the official GitHub runner with a GitHub token for comments and commits.
Only repository writers may invoke it on a validated release-note PR. Its model
can read files and edit the version-specific note; shell and other tools are
denied. A trusted pre-push hook outside the checkout restricts every outgoing
commit to the note file and scans the complete outgoing history for credentials.
The independent gate rejects changes outside that note and never trusts
the model to grant approval. Repository differences and PR context go to your
selected Go model. Signing keys remain available only to the macOS signing step.

The official GitHub runner is invoked directly from `opencode-ai@1.19.5`, avoiding
the official composite action’s automatic download of the latest CLI. Existing
`OPENCODE_GO_API_KEY` and `OPENCODE_MODEL` settings are reused; no new secret or
GitHub App installation is required. See [official GitHub integration](https://opencode.ai/docs/github/).
After merging this migration, existing notes PRs can use `/oc` immediately;
there is no need to recreate their tags or PRs.

### Optional local asset preparation

Outside CI, `release-notes/<tag>.md` and `TEMPLATE.md` remain available for manual
asset preparation. `RELEASE_SUMMARY_FILE` overrides this with a generated artifact.
Local preparation does not call an AI service. If notes change after signing,
prepare/sign the feed again. Editing the GitHub Release body alone does not
change notes already embedded in its appcast.

## GitHub Release and complete commit history

Each GitHub Release body starts with the same approved summary, followed by
an automatically generated history saved to `dist/release-notes.md` during
publication. The history includes:

- The release tag's full commit ID and a commit link.
- A comparison link to the previous latest published release.
- Every commit in `previous_release..current_tag`, including merged commits.
- Each commit's subject (the first line of its commit message) and full linked ID.

The history is an exact commit summary, not an AI-generated paraphrase. Commit subjects
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
startup, automatic-check settings across relaunches, and the real signed-feed
reminder flow: scheduled discovery stays silent, an explicit action opens the
Sparkle window with rendered Markdown (including Chinese and XML special
characters), and dismissal clears the pending version. It does not
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

### Production v0.1.5 → v0.1.6 verification (2026-10-30)

Passed on Apple Silicon, macOS 27.2 (26B123), using the public GitHub Releases
feed and permanent release key. Both tags point to `9d5c5a9`; v0.1.6 changes
the built version only. Its release workflow completed successfully.

- The user downloaded v0.1.5 from GitHub in Chrome and installed it through
  Finder over the existing app. Initial opening required the system security
  approval flow. The installed app retained its Chrome quarantine attribute;
  its executable SHA-256 matched the official v0.1.5 ZIP.
- To exercise the automatic scheduler without waiting 24 hours, we quit the
  app, saved its previous `SULastCheckTime`, set that timestamp two days back,
  and reopened it. No manual Check for Updates action was used. Sparkle replaced
  the test timestamp with the actual check time; the daily interval and
  automatic-check setting were unchanged.
- The user confirmed the v0.1.6 notification appeared automatically. After
  accepting the update, the app quit and relaunched as v0.1.6, with no additional
  Gatekeeper prompt, password, or Touch ID request.
- Sparkle logs confirmed valid EdDSA signatures for both the appcast and ZIP.
  The updated executable matched the official v0.1.6 ZIP, and deep/strict
  codesign verification passed. Signing remained ad-hoc, with no Team ID.
- The restarted process ran from `/Applications/Codex Resets.app`; another
  normal quit/open also started successfully. Quarantine was absent after
  Sparkle installation; no test command removed it or disabled Gatekeeper.

Local evidence is in ignored `build/release-validation-v0.1.5-to-v0.1.6/`.
This verifies this machine's upgrade flow, not every macOS version or a fresh
machine's trust state. Sparkle also logged that this menu-bar app has no gentle
reminder integration; the notification was visible in this test, but background
update prompts may be less noticeable in other usage contexts.

## References

- [Sparkle signing and setup](https://sparkle-project.org/documentation/)
- [Sparkle publishing](https://sparkle-project.org/documentation/publishing/)
- [GitHub release creation](https://cli.github.com/manual/gh_release_create)
- [GitHub release publication](https://cli.github.com/manual/gh_release_edit)
- [GitHub runner labels](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
