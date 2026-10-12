# Branch log: chore/merge-skill-releases

## Merged 2026-10-13

Merged into `main` through a pull request, with a merge commit. The
`/merge-to-main` skill gains a release mode: asked for a release, it bumps the
version, builds and checks the installer and zip, tags the merge commit and
publishes the GitHub release, following what the 1.0.0 release did by hand.

### Commits

| Commit | Subject |
| --- | --- |
| `c94fd3f` | Add release mode to the merge-to-main skill |
| (this commit) | Document chore/merge-skill-releases for the main merge |

### Files

| File | Change |
| --- | --- |
| `.claude/skills/merge-to-main/SKILL.md` | modified, +166 -10 |
| `docs/chore-merge-skill-releases-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 98 tests (app 9, dsp 25 + 2 no-alloc, engine 48, presets 14) |
| Release mode | not run yet: it is used at the next release. This branch landed through the skill's normal mode. |

### Notes

- Release mode is opt-in ("/merge-to-main release X.Y.Z", "create a release
  for version 2"); the normal run is unchanged.
- It writes down what the 1.0.0 release taught: an icon taken from the exe
  doubled the MSI (now caught by a size check), the uninstall product code
  must come from the Uninstall registry key rather than the install log,
  `autocrlf` can leave `Cargo.lock` looking modified after a pull, and a
  published tag is never moved - a fix after publishing is a patch release.
- The install test needs the user's admin approval, so it is always asked
  for, and only offered when packaging changed since the previous release.
