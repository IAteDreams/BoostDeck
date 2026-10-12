# Branch log: chore/merge-skill-pr-flow

## Merged 2026-10-13

Merged into `main` through a pull request, with a merge commit. The
`/merge-to-main` skill now lands branches the way GitHub's new ruleset on
`main` requires - push the branch, open a pull request, merge it - instead of
pushing `main` directly, which the ruleset now refuses.

### Commits

| Commit | Subject |
| --- | --- |
| `194f9f6` | Land branches through a pull request in the merge-to-main skill |
| (this commit) | Document chore/merge-skill-pr-flow for the main merge |

### Files

| File | Change |
| --- | --- |
| `.claude/skills/merge-to-main/SKILL.md` | modified, +87 -46 |
| `docs/chore-merge-skill-pr-flow-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 98 tests (app 9, dsp 25 + 2 no-alloc, engine 48, presets 14) |
| The skill itself | this branch is the first one landed by the new version: its own Phase 4 opens and merges the PR |

### Notes

- Why: feat/package's push to `main` was refused with GH013 ("Cannot update
  this protected ref"). The ruleset then also had "Restrict updates", which
  blocked PR merges too; the user replaced it with "Require a pull request"
  (no approvals), keeping "Restrict deletions" and "Block force pushes".
- Merges use a merge commit on purpose: squash and rebase would rewrite the
  branch's commits, and each branch log names them by sha.
- The skill now requires reading every push and merge exit status: during
  feat/package a rejected push was hidden behind `| tail`, and the branch was
  deleted after it (nothing was lost; the commits were on local `main`).
- Never `--admin` or any other bypass: the ruleset is the user's to change.
