# Branch log: chore/merge-to-main-skill

## Merged 2026-10-12

Merged into `main` as a fast-forward. Adds a project-level skill that lands a
feature branch on `main`: it checks the preconditions, runs the cargo gate,
writes a log like this one, fast-forwards, pushes, and deletes the branch. The
flow was being driven by hand until now.

### Commits

| Commit | Subject |
| --- | --- |
| `2ae0263` | Add the merge-to-main skill |

### Files

| File | Change |
| --- | --- |
| `.claude/skills/merge-to-main/SKILL.md` | added, +174 -0 |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass, no warnings |
| `cargo test --workspace` | pass, 7 tests |
| Skill guard commands run against this repo | `git merge-base --is-ancestor` returned 0 for an ancestor and 1 for a non-ancestor; `git ls-remote --exit-code --heads` returned 0 for an existing branch and 2 for a missing one |
| Landing this branch with the skill's own procedure | every phase behaved as written; this log is the Phase 3 artifact |

### Notes

The branch log is committed on the feature branch, not on `main`. Committing it
to `main` first would leave `main` holding a commit the branch does not have,
and the `--ff-only` merge in Phase 4 would fail. The question that prompted
this skill proposed the opposite order; this is the correction.

The skill could not be invoked through the `Skill` tool in the session that
created it - skills are registered when a session starts, so a mid-session
addition reports `Unknown skill`. Its phases were executed by hand instead,
which is what the Verification table above records. It should be callable
normally from the next session.

The gate is this workspace's `cargo` invocation, which is why the skill is
project-level (`.claude/skills/`, committed) rather than user-level. A
different project would need its own gate.
