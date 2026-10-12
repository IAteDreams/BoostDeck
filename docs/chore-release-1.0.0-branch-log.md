# Branch log: chore/release-1.0.0

## Merged 2026-10-13

Merged into `main` through a pull request, with a merge commit. BoostDeck's
version moves to 1.0.0 for its first release, and the installer shrinks back
to 5.2 MB after an earlier fix had packed a second copy of the program into
it.

### Commits

| Commit | Subject |
| --- | --- |
| `9326ae8` | Release 1.0.0 |
| `1437cb2` | Keep the exe out of the MSI's icon table |
| (this commit) | Document chore/release-1.0.0 for the main merge |

### Files

| File | Change |
| --- | --- |
| `Cargo.lock` | modified, +4 -4 |
| `Cargo.toml` | modified, +1 -1 |
| `packaging/boostdeck.wxs` | modified, +26 -13 |
| `packaging/package.ps1` | modified, +1 -1 |
| `docs/chore-release-1.0.0-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 98 tests (app 9, dsp 25 + 2 no-alloc, engine 48, presets 14) |
| Release `boostdeck.exe` | FileVersion and ProductVersion 1.0.0; the log's first line reads "BoostDeck 1.0.0 starting" |
| `packaging/package.ps1` | `BoostDeck-1.0.0-portable.zip` 6.1 MB, `BoostDeck-1.0.0-x64.msi` 5.2 MB (20 MB before the icon fix) |
| `wix msi validate` | clean, no warnings |
| `msiexec /a` extraction | exe byte-identical to the release build |
| Install on this PC (`msiexec /i`, admin approved by the user) | "Installation completed successfully"; three files in `C:\Program Files\BoostDeck`; Start menu shortcut to `boostdeck.exe` with the exe's icon; "BoostDeck 1.0.0" in Apps & features |
| Run from the Start menu shortcut | started from `C:\Program Files\BoostDeck\boostdeck.exe` with the user's settings and presets |
| Uninstall (`msiexec /x`) | folder, shortcut and Apps & features entry removed; settings and presets kept |

### Notes

- feat/package's ICE50 fix took the shortcut icon from `boostdeck.exe`; the
  MSI Icon table stores the whole source file, so the 0.1.0 MSI the user
  tested on the second PC was about 20 MB. 1.0.0 takes the Apps & features
  icon from the 23 KB `.ico`, and the Start menu shortcut is a plain one in
  its own component keyed by an HKCU registry value (ICE43, ICE57), pointing
  at `[INSTALLFOLDER]boostdeck.exe` (ICE69). It shows the exe's own icon.
- The plain shortcut leaves one HKCU value, `Software\BoostDeck\StartMenuShortcut`,
  for the installing user; uninstall removes it with the component.
- Release artifacts are built from this branch's tree, which the merge
  commit carries unchanged; the `v1.0.0` tag goes on that merge commit.
