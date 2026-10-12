# Branch log: docs/readme-release-process

## Merged 2026-10-13

Merged into `main` through a pull request, with a merge commit. The README now
explains how changes and releases reach `main` - through pull requests landed
by `/merge-to-main`, and releases through its release mode - and its status
banner points at the 1.0.0 release instead of saying the project has not
started.

### Commits

| Commit | Subject |
| --- | --- |
| `d1b07f8` | Document the pull-request and release process in the README |
| (this commit) | Document docs/readme-release-process for the main merge |

### Files

| File | Change |
| --- | --- |
| `README.md` | modified, +73 -2 |
| `docs/docs-readme-release-process-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 98 tests (app 9, dsp 25 + 2 no-alloc, engine 48, presets 14) |
| README links | the new anchors (`#setting-up-vb-cable`, `#build-and-test`, `#coding-order`) match existing headings |

### Notes

- The new section is "Branches, pull requests and releases", after
  Packaging. It describes the skill's steps and the equivalent `git`/`gh`
  commands, so the process works without Claude Code too.
- The status banner had said "pre-implementation, no crates are committed
  yet" since step 1.
