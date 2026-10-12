# Branch log: feat/package

## Merged 2026-10-13

Merged into `main` as a fast-forward. BoostDeck now ships: one script builds
an unsigned portable zip and an MSI installer, the app carries its own icon
and version information, and every run leaves a log file behind
(coding-order step 11, the last one).

### Commits

| Commit | Subject |
| --- | --- |
| `2a685ec` | Package BoostDeck: icon, version info, log files, portable zip and MSI |
| (this commit) | Document feat/package for the main merge |

### Files

| File | Change |
| --- | --- |
| `.gitignore` | modified, +3 -0 |
| `Cargo.lock` | modified, +75 -0 |
| `Cargo.toml` | modified, +2 -1 |
| `README.md` | modified, +18 -7 |
| `assets/.gitkeep` | deleted |
| `assets/boostdeck-256.png` | added (binary) |
| `assets/boostdeck.ico` | added (binary) |
| `assets/make-icon.ps1` | added, +56 -0 |
| `crates/app/Cargo.toml` | modified, +4 -0 |
| `crates/app/build.rs` | added, +69 -0 |
| `crates/app/src/logging.rs` | added, +149 -0 |
| `crates/app/src/main.rs` | modified, +23 -21 |
| `packaging/README.txt` | added, +58 -0 |
| `packaging/boostdeck.wxs` | added, +56 -0 |
| `packaging/package.ps1` | added, +43 -0 |
| `docs/feat-package-branch-log.md` | added |

### Verification

| Check | Result |
| --- | --- |
| `cargo build` | pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | pass |
| `cargo test --workspace` | pass, 98 tests (app 9, dsp 25 + 2 no-alloc, engine 48, presets 14) |
| Release `boostdeck.exe` | 14.8 MB; FileDescription, ProductName, FileVersion 0.1.0 and the licence embedded; no console (0 bytes to stdout); icon in the title bar |
| Log file | written to `%LOCALAPPDATA%\BoostDeck\logs`, local time matching the clock; pruning to the newest 5 covered by a unit test |
| `packaging/package.ps1` | `dist/BoostDeck-0.1.0-portable.zip` (6.1 MB: exe, README.txt, LICENSE.txt) and `dist/BoostDeck-0.1.0-x64.msi` (5.2 MB) |
| `wix msi validate` | clean, after fixing ICE50 (the shortcut icon must come from the exe, not the `.ico`) |
| `msiexec /a` extraction | exactly `PFiles64/BoostDeck/boostdeck.exe`, `LICENSE.txt`, `README.txt`; the exe byte-identical to the release build |
| Step 11 "done when", by the user | installs and runs on a second PC with VB-Cable: "It works" |

### Notes

- Step 11 is ticked in the README: all eleven coding-order steps are done.
- Deviation from the README's plan: the MSI is built from a hand-written
  `packaging/boostdeck.wxs` with the WiX Toolset v5 (`dotnet tool install
  --global wix --version 5.0.2`, installed on this PC with the user's
  approval) rather than cargo-wix, whose WiX v3 needs admin rights and the
  .NET 3.5 runtime. v5 is the last MS-RL release; v6 onwards asks for a
  maintenance-fee licence. The README's Packaging section is updated.
- The `.wxs` file's `UpgradeCode` (`7EF8AD7F-4FF5-47A9-B365-BAE0315508C3`)
  must never change: it is how a newer MSI replaces an older one.
- The MSI installs per machine into Program Files (one UAC prompt);
  settings, presets and logs stay in the user's profile and survive an
  uninstall.
- The MSI was not installed on this PC; the second-PC install is the
  installed test. Whether that used the MSI or the zip, and whether the
  SmartScreen warning showed, was not recorded.
- Log times use simplelog's `local-offset` feature, which must be set before
  any other thread starts; logging is the first thing `main` does.
- Builds stay unsigned, so first runs show "Windows protected your PC", and
  Smart App Control blocks the installer; both are covered in the shipped
  README.txt.
