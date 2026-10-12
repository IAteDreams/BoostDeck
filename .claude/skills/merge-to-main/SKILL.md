---
name: merge-to-main
description: Finish a feature branch - run the verification gate, write a per-branch log to docs/, push the branch, open a pull request into main and merge it with a merge commit, then sync main and delete the branch. Use when the user says to merge a branch, merge into main, land a branch, finish/wrap up a branch, or asks to merge-document-delete. Only ever targets main, never pushes to main directly, never force-pushes, and never bypasses branch protection.
---

# Merge a feature branch into main

Lands the current feature branch on `main` through a pull request, with a
documented record, then removes the branch. Five phases, in order. **Stop and
report at the first failed precondition** - never work around one.

`main` on GitHub is protected by a ruleset: every change goes through a pull
request, and force pushes and deletion are blocked. So this skill never pushes
to `main`; it pushes the branch and merges a PR. If the user names a different
merge target, say this skill does not do that and stop.

**Read every exit status.** Run each push, PR and merge command on its own, or
check `$?` / `PIPESTATUS` explicitly. Never pipe one through `| tail` in a chain
that carries on regardless: a rejected push once went unnoticed that way and the
branch was deleted after it.

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
| Current branch | it is `main` | Nothing to land; ask which branch they meant |
| Working tree | `git status --porcelain` prints anything | Commit or stash first; name the files |
| Sync | `main` and `origin/main` differ | Local `main` has drifted from GitHub's; ask before touching it |
| Ancestry | `--is-ancestor` exits non-zero | The branch is behind `main`; bringing it up to date (rebase or merge) is the user's call |
| Commits | `origin/main..<branch>` is empty | The branch adds nothing |
| GitHub login | `gh auth status` reports no valid login | Ask them to run `! gh auth login` in this session |
| Existing PR | a PR for the branch is `MERGED` or `CLOSED` | Something already happened to it; report it and ask |

An `OPEN` PR for the branch is fine: Phase 4 reuses it instead of opening a
second one.

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
claiming a pass.

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
file rather than quoting it inline.

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
```

`gh pr merge --delete-branch` normally deletes both the remote branch and the
local one; `git branch -d` is for when it did not. Lowercase `-d` only, which
refuses to delete anything unmerged. If it refuses, or `git pull --ff-only`
fails, the merge did not land the way it should have: investigate rather than
forcing with `-D` or resetting.

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
