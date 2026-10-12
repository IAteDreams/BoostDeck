---
name: merge-to-main
description: Finish a feature branch - run the verification gate, write a per-branch log to docs/, fast-forward main, push, and delete the branch. Use when the user says to merge a branch, merge into main, land a branch, finish/wrap up a branch, or asks to merge-document-delete. Only ever merges into main, only ever fast-forwards, and never force-pushes.
---

# Merge a feature branch into main

Lands the current feature branch on `main` with a documented record, then
removes the branch. Five phases, in order. **Stop and report at the first
failed precondition** - never work around one.

Target is always `main`. If the user names a different merge target, say this
skill does not do that and stop.

## Phase 1: preconditions

Run these together and read every answer before acting:

```sh
git rev-parse --abbrev-ref HEAD        # the branch to land
git status --porcelain                 # must be empty
git fetch origin
git rev-parse main origin/main         # must be identical
git merge-base --is-ancestor main <branch>; echo $?   # must be 0
git log --oneline main..<branch>       # must be non-empty
```

Abort, with the reason and the fix, if:

| Check | Abort when | What to tell the user |
| --- | --- | --- |
| Current branch | it is `main` | Nothing to land; ask which branch they meant |
| Working tree | `git status --porcelain` prints anything | Commit or stash first; name the files |
| Sync | `main` and `origin/main` differ | `main` has diverged from the remote; ask before touching it |
| Ancestry | `--is-ancestor` exits non-zero | Not fast-forwardable; a rebase is needed, which is the user's call |
| Commits | `main..<branch>` is empty | The branch adds nothing |

Never resolve a diverged `main` or a failed ancestry check on your own
initiative: no `rebase`, no `merge` without `--ff-only`, no `push --force`,
no `reset --hard`. Report and ask.

## Phase 2: verification gate

Run the project gate on the branch, before anything is merged:

```sh
cargo build
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Record the real results - they go in the log. **If any of them fails, stop and
report.** Do not merge failing code, and do not weaken the gate to get past it.

Skip this phase only if the user passed `--no-verify` or explicitly said to
skip it; then write "skipped at the user's request" in the log rather than
claiming a pass.

## Phase 3: write the branch log

One file per branch, at `docs/<slug>-branch-log.md`, where `<slug>` is the
branch name with `/` replaced by `-` (`feat/device-list` ->
`docs/feat-device-list-branch-log.md`).

Gather the facts first - never write the log from memory:

```sh
git log --oneline --no-decorate main..<branch>
git diff --stat main..<branch>
git diff --name-status main..<branch>
```

Write the file to this shape. If it already exists, append a new
`## Merged <date>` section rather than overwriting what is there.

```markdown
# Branch log: <branch>

## Merged <YYYY-MM-DD>

Merged into `main` as a fast-forward. <one sentence on what the branch
delivers, in plain words - what changed for the project, not a restatement
of the commit subjects.>

### Commits

| Commit | Subject |
| --- | --- |
| `<sha>` | <subject> |

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
check was skipped, say skipped. Never record a result you did not see.

Then commit it **on the feature branch** - this is what keeps the merge a
fast-forward:

```sh
git add docs/<slug>-branch-log.md
git commit -m "Document <branch> for the main merge"
```

Committing the log on `main` instead would put a commit on `main` that the
branch does not have, and `--ff-only` in Phase 4 would then fail.

## Phase 4: merge and push

```sh
git checkout main
git merge --ff-only <branch>
git push origin main
```

`--ff-only` is not negotiable. If it fails here, something changed since
Phase 1: re-run the Phase 1 checks, report, and stop. Do not reach for
`--no-ff`, `--force`, or a rebase.

## Phase 5: delete the branch

```sh
git branch -d <branch>
```

Lowercase `-d` only, which refuses to delete anything unmerged. If it refuses,
the merge did not land - investigate rather than forcing with `-D`.

Delete the remote branch too, but only if it was ever pushed:

```sh
git ls-remote --exit-code --heads origin <branch> && git push origin --delete <branch>
```

## Report

Confirm the end state from the repo, not from assumption:

```sh
git status --short --branch
git log --oneline --decorate -3
git branch -a
```

Then tell the user, briefly:

- the sha `main` now points at, and the fast-forward range (`<old>..<new>`)
- that the push landed, and the branch is gone
- the path of the branch log that was written
- anything from the Notes section they should know about

If the project tracks progress in a checklist (BoostDeck's README has an
11-step coding order), tick off anything this branch completed, as part of
the branch's own work before Phase 3 - not as a separate commit on `main`.
