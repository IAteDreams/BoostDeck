//! Compiles the app icon and version information into `boostdeck.exe`, so
//! Explorer, the taskbar and the installer show BoostDeck's icon and name.
//!
//! The resource script is written into `OUT_DIR` with the icon's absolute
//! path, so it does not depend on the directory the resource compiler runs
//! in.

use std::path::Path;

fn main() {
    let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo");
    let out_dir = std::env::var("OUT_DIR").expect("set by cargo");
    let icon = Path::new(&manifest_dir).join("../../assets/boostdeck.ico");
    let icon = icon
        .canonicalize()
        .unwrap_or_else(|e| panic!("finding {}: {e}", icon.display()));
    println!("cargo:rerun-if-changed={}", icon.display());

    let version = env!("CARGO_PKG_VERSION");
    let numeric = version
        .split(['.', '-'])
        .take(3)
        .map(|part| part.parse::<u16>().unwrap_or(0).to_string())
        .collect::<Vec<_>>()
        .join(",");
    // rc.exe takes C-style strings: backslashes doubled. The `\\?\` prefix
    // from `canonicalize` is not a path rc understands.
    let icon_path = icon
        .display()
        .to_string()
        .trim_start_matches(r"\\?\")
        .replace('\\', "\\\\");

    let script = format!(
        r#"1 ICON "{icon_path}"

1 VERSIONINFO
FILEVERSION {numeric},0
PRODUCTVERSION {numeric},0
FILEOS 0x40004
FILETYPE 0x1
BEGIN
  BLOCK "StringFileInfo"
  BEGIN
    BLOCK "040904B0"
    BEGIN
      VALUE "CompanyName", "BoostDeck"
      VALUE "FileDescription", "BoostDeck"
      VALUE "FileVersion", "{version}"
      VALUE "InternalName", "boostdeck"
      VALUE "LegalCopyright", "MIT License"
      VALUE "OriginalFilename", "boostdeck.exe"
      VALUE "ProductName", "BoostDeck"
      VALUE "ProductVersion", "{version}"
    END
  END
  BLOCK "VarFileInfo"
  BEGIN
    VALUE "Translation", 0x409, 1200
  END
END
"#
    );
    let rc = Path::new(&out_dir).join("boostdeck.rc");
    std::fs::write(&rc, script).expect("writing the resource script");
    embed_resource::compile(&rc, embed_resource::NONE)
        .manifest_optional()
        .expect("compiling the resource script");
}
