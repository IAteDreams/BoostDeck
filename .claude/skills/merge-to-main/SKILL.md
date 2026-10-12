---
name: merge-to-main
description: Finish a feature branch - run the verification gate, write a per-branch log to docs/, push the branch, open a pull request into main and merge it with a merge commit, then sync main and delete the branch. In release mode it also bumps the version, builds and verifies the MSI and portable zip, tags the merge commit and publishes a GitHub release with both attached. Use when the user says to merge a branch, merge into main, land a branch, finish/wrap up a branch, or asks to merge-document-delete; use release mode when they ask to release, tag or publish a version (e.g. "/merge-to-main release 1.1.0"). Only ever targets main, never pushes to main directly, never force-pushes, never moves a pushed tag, and never bypasses branch protection.
---

# Merge a feature branch into main

Lands the current feature branch on `main` through a pull request, with a
documented record, then removes the branch. Five phases, in order, plus
release mode when the user asks for a release. **Stop and report at the
first failed precondition** - never work around one.

`main` on GitHub is protected by a ruleset: every change goes through a pull
request, and force pushes and deletion are blocked. So this skill never pushes
to `main`; it pushes the branch and merges a PR. If the user names a different
merge target, say this skill does not do that and stop.

**Read every exit status.** Run each push, PR, tag, release and merge command
on its own, or check `$?` / `PIPESTATUS` explicitly. Never pipe one through
`| tail` in a chain that carries on regardless: a rejected push once went
unnoticed that way and the branch was deleted after it.

## Release mode

Use it when the user asks to release, tag or publish a version - "release
1.1.0", "create a release for version 2", "tag v1.0.1". Everything below still
applies; the steps marked **Release mode** are added. Without a release
request, skip them all.

- **The version** is `X.Y.Z`, and the tag `vX.Y.Z`. Read loose wording the
  semver way ("version 1" is `1.0.0`, "a patch release" bumps Z) and say in the
  report how you read it. It must be greater than the current
  `[workspace.package] version` in `Cargo.toml`.
- **The branch.** Release from a branch named `chore/release-X.Y.Z`. If the user
  is on `main` with nothing else to land, create it there. If they are on a
  feature branch, land that first as a normal run, then release from a fresh
  branch off the updated `main`, so the release PR holds only the release.
- **The version bump is part of the release.** The exe's version information,
  the MSI and the packaged file names all come from `Cargo.toml`. A tag on a
  commit that still says the old version is wrong; never tag without the bump.

## Phase 1: preconditions

Run these together and read every answer before acting:

```sh
git rev-parse --abbrev-ref HEAD        # the branch to land
git status --porcelain                 # must be empty
git fetch origin
git rev-parse main origin/main         # must be identical
git merge-base --is-ancestor origin/main <branch>; echo $?   # must be 0
git log --oneline origin/main..<branch>    # must be non-empty
gh auth status                         # must say "Logged in"
gh pr list --head <branch> --state all --json number,state,url
```

Abort, with the reason and the fix, if:

| Check | Abort when | What to tell the user |
| --- | --- | --- |
| Current branch | it is `main` (outside release mode) | Nothing to land; ask which branch they meant |
| Working tree | `git status --porcelain` prints anything | Commit or stash first; name the files |
| Sync | `main` and `origin/main` differ | Local `main` has drifted from GitHub's; ask before touching it |
| Ancestry | `--is-ancestor` exits non-zero | The branch is behind `main`; bringing it up to date (rebase or merge) is the user's call |
| Commits | `origin/main..<branch>` is empty | The branch adds nothing |
| GitHub login | `gh auth status` reports no valid login | Ask them to run `! gh auth login` in this session |
| Existing PR | a PR for the branch is `MERGED` or `CLOSED` | Something already happened to it; report it and ask |

An `OPEN` PR for the branch is fine: Phase 4 reuses it instead of opening a
second one.

**Release mode** - a fresh `chore/release-X.Y.Z` has no commits until the
version bump below, so run the Commits check after the bump. Also check, and
abort on any failure:

```sh
grep -n '^version' Cargo.toml                      # current version, must be lower
git tag -l vX.Y.Z                                  # must print nothing
git ls-remote --exit-code --tags origin vX.Y.Z; echo $?   # must be 2 (no such tag)
gh release view vX.Y.Z; echo $?                    # must fail: no such release
PATH="$PATH:$USERPROFILE/.dotnet/tools" wix --version     # WiX Toolset v5
```

A tag or release that already exists is never replaced: report it and ask. If
`wix` is missing, the fix is `dotnet tool install --global wix --version 5.0.2`
(README "Packaging") - installing it is the user's call; ask first.

Then, on `chore/release-X.Y.Z`, bump the version and commit it:

```sh
# Cargo.toml, [workspace.package]: version = "X.Y.Z"
cargo build                       # refreshes the four crates' versions in Cargo.lock
git commit -am "Release X.Y.Z"
```

Search for the old version elsewhere (`grep -rn` outside `target/`, `dist/`
and `docs/`): branch logs are history and stay as they are, but anything a user
sees must not still say the old version.

Never resolve a drifted `main` or a branch that is behind on your own
initiative: no `rebase`, no `push --force`, no `reset --hard`. Report and ask.

## Phase 2: verification gate

Run the project gate on the branch, before anything is merged:

```sh
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Record the real results - they go in the log. **If any of them fails, stop and
report.** Do not merge failing code, and do not weaken the gate to get past it.
On Windows, "Access is denied" from `cargo build` usually means a BoostDeck
window is still open and locks the `.exe`: ask the user to close it, never kill
it yourself - it may be boosting their audio.

Skip this phase only if the user passed `--no-verify` or explicitly said to
skip it; then write "skipped at the user's request" in the log rather than
claiming a pass. Release mode never skips it.

**Release mode** - build the packages from the branch and verify them before
anything is merged; these are the exact files that get published:

```sh
export PATH="$PATH:$USERPROFILE/.dotnet/tools"
rm -rf dist
powershell -NoProfile -ExecutionPolicy Bypass -File packaging/package.ps1
```

| Check | How | Must be |
| --- | --- | --- |
| Version | `(Get-Item target/release/boostdeck.exe).VersionInfo` | FileVersion and ProductVersion `X.Y.Z` |
| Files | `ls -la dist/` | `BoostDeck-X.Y.Z-x64.msi` and `BoostDeck-X.Y.Z-portable.zip` |
| Validation | `wix msi validate dist/BoostDeck-X.Y.Z-x64.msi` | no errors and no warnings; fix the `.wxs` rather than ship one |
| Contents | `msiexec /a <msi> /qn TARGETDIR=<scratch dir>`, then list it | exactly `boostdeck.exe`, `LICENSE.txt`, `README.txt` under `PFiles64/BoostDeck`, the exe byte-identical (`cmp`) to `target/release/boostdeck.exe` |
| Size | compare the MSI to the exe | a little smaller than the exe (it is compressed). An MSI larger than the exe carries something twice: the `Icon` table stores its whole source file, so an icon taken from the exe once made the MSI 20 MB |
| Checksums | `sha256sum dist/*.msi dist/*.zip` | recorded for the release notes |

**Install test.** If anything under `packaging/`, `assets/` or
`crates/app/build.rs` changed since the previous release tag
(`git diff --quiet <previous tag> HEAD -- packaging assets crates/app/build.rs`),
or there is no previous release, the installer has never been installed in
this form. Ask the user (AskUserQuestion) before installing: it needs their
admin approval at a UAC prompt. If they agree:

1. `msiexec /i <msi> /qb /l*v <scratch>\install.log`, then check the log says
   "Installation completed successfully".
2. The three files are in `C:\Program Files\BoostDeck`; the Start menu
   shortcut (`C:\ProgramData\Microsoft\Windows\Start Menu\Programs\BoostDeck.lnk`)
   targets `boostdeck.exe`; Apps & features (`HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*`)
   lists "BoostDeck" at `X.Y.Z`.
3. Launch it from the shortcut: the process path is under Program Files and
   the newest log in `%LOCALAPPDATA%\BoostDeck\logs` starts "BoostDeck X.Y.Z
   starting". Close it the normal way (`taskkill` without `/F`).
4. Uninstall with the product code read from the Uninstall registry key's
   name (`PSChildName`) - not from the install log, whose first
   `ProductCode` match is not this product's and fails with 1605:
   `msiexec /x <code> /qb`. Then confirm the folder, shortcut and Apps &
   features entry are gone and the user's settings and presets are still in
   `%APPDATA%\BoostDeck\BoostDeck\config`.

If the user declines, say in the branch log and in the report that this MSI
was validated but not installed.

## Phase 3: write the branch log

If the project tracks progress in a checklist (BoostDeck's README has an
11-step coding order), tick off anything this branch completed first, so the
tick is part of the branch.

One file per branch, at `docs/<slug>-branch-log.md`, where `<slug>` is the
branch name with `/` replaced by `-` (`feat/device-list` ->
`docs/feat-device-list-branch-log.md`).

Gather the facts first - never write the log from memory:

```sh
git log --oneline --no-decorate origin/main..<branch>
git diff --stat origin/main..<branch>
git diff --name-status origin/main..<branch>
```

Write the file to this shape. If it already exists, append a new
`## Merged <date>` section rather than overwriting what is there.

```markdown
# Branch log: <branch>

## Merged <YYYY-MM-DD>

Merged into `main` through a pull request, with a merge commit. <one sentence
on what the branch delivers, in plain words - what changed for the project,
not a restatement of the commit subjects.>

### Commits

| Commit | Subject |
| --- | --- |
| `<sha>` | <subject> |
| (this commit) | Document <branch> for the main merge |

### Files

| File | Change |
| --- | --- |
| `<path>` | <added / modified / deleted>, +<n> -<m> |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | <pass / fail / skipped> |
| `cargo clippy --workspace --all-targets -- -D warnings` | <result> |
| `cargo test --workspace` | <result, with the test count> |
| <any manual check that was run> | <what was observed> |

### Notes

<Only when there is something a reader needs later: a deviation from the
build guide, a decision taken and why, a follow-up left open, a surprise
found while running the code. Omit the section entirely if there is none -
do not pad it.>
```

Facts only. The verification table records what actually ran in Phase 2; if a
check was skipped, say skipped. Never record a result you did not see. The PR
number is not known yet, so the log does not name it; the report does.

**Release mode**: the verification table also carries every package check
from Phase 2 and the install test (or that it was declined), and the Notes say
the `vX.Y.Z` tag goes on the merge commit.

Then commit it **on the feature branch**, so the PR carries it:

```sh
git add docs/<slug>-branch-log.md
git commit -m "Document <branch> for the main merge"
```

## Phase 4: pull request and merge

Push the branch, on its own:

```sh
git push -u origin <branch>
```

Open the PR, unless Phase 1 found an open one to reuse:

```sh
gh pr create --base main --head <branch> --title "<the branch's main commit subject>" --body-file <file>
```

The body: the log's opening sentence, a short "What's in it" list, the
verification results, a pointer to `docs/<slug>-branch-log.md`, and the
attribution line the session asks for on PR descriptions. Write it to a temp
file rather than quoting it inline. In release mode the title is
`Release X.Y.Z`.

Check that GitHub will take it, then merge:

```sh
gh pr view <n> --json state,mergeable,mergeStateStatus,reviewDecision
gh pr merge <n> --merge --delete-branch
```

Merge only when `mergeable` is `MERGEABLE` and `mergeStateStatus` is `CLEAN`
(or `HAS_HOOKS`). Anything else - `BLOCKED`, `BEHIND`, `DIRTY`, `UNSTABLE`,
required reviews or checks still pending - means stop, leave the PR open, and
report what GitHub is waiting for. For `BLOCKED` with no checks or reviews
pending, read the ruleset to name the rule:
`gh api repos/<owner>/<repo>/rules/branches/main`. The ruleset is the user's to
change, never yours.

`--merge` (a merge commit) is deliberate: squash and rebase merges rewrite the
branch's commits, and the log names them by sha. Never pass `--admin`, `--auto`,
`--squash` or `--rebase`, and never try to push `main` directly.

## Phase 5: sync and clean up

```sh
git checkout main
git pull --ff-only origin main
git fetch --prune origin
git branch -d <branch>     # only if it still exists locally
git status --short         # must be empty
```

`gh pr merge --delete-branch` normally deletes both the remote branch and the
local one; `git branch -d` is for when it did not. Lowercase `-d` only, which
refuses to delete anything unmerged. If it refuses, or `git pull --ff-only`
fails, the merge did not land the way it should have: investigate rather than
forcing with `-D` or resetting.

`core.autocrlf` is on, so the pull can leave a file (usually `Cargo.lock`)
showing as modified with nothing but line endings changed. Confirm `git diff
<file>` shows no content change, then `git checkout -- <file>`. If it shows a
content change, stop and report: something else wrote to it.

**Release mode** - confirm the merged tree is the one the packages were built
from, apart from the branch log:

```sh
git diff --stat <last commit before the log> HEAD -- . ':!docs'    # must print nothing
```

If it prints anything, `main` changed under the release: stop, do not tag,
and report.

## Phase 6 (release mode): tag and publish

Tag the merge commit - the commit `main` now points at - with an annotated
tag, and push the tag on its own (the ruleset covers `main`, not tags):

```sh
git tag -a vX.Y.Z <merge commit> -m "BoostDeck X.Y.Z" -m "<one or two lines on what this release is>"
git push origin vX.Y.Z
```

Write the release notes to a temp file:

- One opening sentence on what BoostDeck is (first release) or what changed
  (later ones: the PRs merged since the previous tag,
  `git log --merges --oneline <previous tag>..vX.Y.Z`).
- **Download**: what the MSI and the zip are each for; the zip is the fallback
  when Smart App Control blocks the installer.
- **Install VB-Cable first**, linked to https://vb-audio.com/Cable/ - it is not
  bundled.
- What is new, or for a first release the feature list.
- **Before you run it**: the builds are unsigned ("Windows protected your PC":
  More info, Run anyway), and the safety note about volume and headphones.
- Known limitations, from the README.
- **Checksums (SHA-256)** for both files, from Phase 2.

Publish with both files, then check what GitHub actually serves:

```sh
gh release create vX.Y.Z dist/BoostDeck-X.Y.Z-x64.msi dist/BoostDeck-X.Y.Z-portable.zip --title "BoostDeck X.Y.Z" --notes-file <file> --verify-tag
gh release view vX.Y.Z --json tagName,name,isDraft,isPrerelease,url,assets
gh release download vX.Y.Z -D <scratch dir>     # then sha256sum: must match dist/
```

Add `--prerelease` only when the user asks for one, or when they declined the
install test and want it marked. A published tag is never moved, deleted or
re-pushed: if something is wrong after publishing, report it and suggest a
patch release (`X.Y.Z+1`), which the user decides on.

## Report

Confirm the end state from the repo and GitHub, not from assumption:

```sh
git status --short --branch
git log --oneline --decorate -4
git branch -a
gh pr view <n> --json state,mergedAt,mergeCommit,url
```

Then tell the user, briefly:

- the PR (number and link) and the merge commit `main` now points at
- that local `main` matches GitHub's, and the branch is gone locally and on GitHub
- the path of the branch log that was written
- anything from the Notes section they should know about

**Release mode** - also: the release link, the tag and the commit it is on,
the two files with their sizes, that the downloaded copies match the built
ones, how the version was read from their wording, and whether the MSI was
install-tested.
