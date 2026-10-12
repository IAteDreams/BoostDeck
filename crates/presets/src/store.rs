//! `PresetStore`: load, save and the operations the UI calls.
//!
//! | Operation | Rule |
//! | --- | --- |
//! | `create(name, gain, output)` | Name trimmed, 1-40 chars, unique ignoring case |
//! | `update(id, gain, output)` | Overwrites a preset's settings ("Save") |
//! | `rename(id, name)` | Same name rules as create |
//! | `duplicate(id)` | New id, name "<name> copy", "<name> copy 2", ... |
//! | `delete(id)` | Confirmed in the UI; the last remaining preset cannot go |
//! | `set_active(id)` | The UI applies it: gain through the smoother, a missing output keeps the current one |
//! | `list()` | Sorted by name |
//!
//! Every change is saved at once. Saving is atomic: write `presets.json.tmp`,
//! flush, copy the old file to `presets.json.bak`, then rename the temp over
//! `presets.json`.
//!
//! On load, a parse failure falls back to `.bak`; if that fails too, the
//! broken file is renamed to `presets.broken-<timestamp>.json` and the
//! built-ins are used instead, with a note to the user.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

use crate::NAME_LEN_RANGE;
use crate::migrate;
use crate::model::{DeviceRef, Preset, PresetFile};

pub const FILE_NAME: &str = "presets.json";

#[derive(Debug, thiserror::Error)]
pub enum PresetError {
    #[error("a preset needs a name")]
    NameEmpty,
    #[error("names are at most {} characters", NAME_LEN_RANGE.1)]
    NameTooLong,
    #[error("there is already a preset called {0:?}")]
    NameTaken(String),
    #[error("that preset no longer exists")]
    NotFound,
    #[error("the last preset cannot be deleted")]
    LastPreset,
    #[error("saving presets failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("saving presets failed: {0}")]
    Json(#[from] serde_json::Error),
}

/// How loading went, for the UI to report when it was not plain sailing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadOutcome {
    /// Read as it was (or migrated from an older version).
    Loaded,
    /// No file yet: the built-ins, written out.
    FirstRun,
    /// `presets.json` was unreadable; the backup was used. The broken file
    /// was kept under the given name.
    FromBackup { broken: PathBuf },
    /// Neither file was usable: the built-ins. The broken file was kept.
    Reset { broken: Option<PathBuf> },
}

impl LoadOutcome {
    /// What to tell the user, if anything.
    pub fn notice(&self) -> Option<String> {
        match self {
            Self::Loaded | Self::FirstRun => None,
            Self::FromBackup { broken } => Some(format!(
                "presets.json was damaged, so your presets were restored from the backup. \
                 The damaged file was kept as {}.",
                file_name(broken)
            )),
            Self::Reset { broken } => Some(match broken {
                Some(broken) => format!(
                    "presets.json and its backup were both damaged, so the built-in presets \
                     are back. The damaged file was kept as {}.",
                    file_name(broken)
                ),
                None => {
                    "Your presets could not be read, so the built-in presets are back.".to_owned()
                }
            }),
        }
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

pub struct PresetStore {
    /// `presets.json`, or `None` when there is nowhere to save: presets then
    /// live for the session only.
    path: Option<PathBuf>,
    file: PresetFile,
}

impl PresetStore {
    /// Loads `dir/presets.json`, falling back as the module docs describe.
    /// Anything it had to repair is written back straight away.
    pub fn load(dir: Option<&Path>) -> (Self, LoadOutcome) {
        let Some(dir) = dir else {
            log::warn!("No app-data folder; presets will not be saved");
            return (
                Self {
                    path: None,
                    file: PresetFile::built_ins(),
                },
                LoadOutcome::FirstRun,
            );
        };
        let path = dir.join(FILE_NAME);
        let (file, outcome) = read_with_fallback(&path);
        let mut store = Self {
            path: Some(path),
            file,
        };
        store.drop_dangling_active();
        if outcome != LoadOutcome::Loaded
            && let Err(error) = store.save()
        {
            log::error!("{error}");
        }
        (store, outcome)
    }

    /// Every preset, sorted by name ignoring case.
    pub fn list(&self) -> Vec<&Preset> {
        let mut presets: Vec<&Preset> = self.file.presets.iter().collect();
        presets.sort_by_key(|p| p.name.to_lowercase());
        presets
    }

    pub fn get(&self, id: Uuid) -> Option<&Preset> {
        self.file.presets.iter().find(|p| p.id == id)
    }

    pub fn active(&self) -> Option<&Preset> {
        self.file.active.and_then(|id| self.get(id))
    }

    /// Marks a preset as the one in use. The UI applies its settings.
    pub fn set_active(&mut self, id: Uuid) -> Result<&Preset, PresetError> {
        self.get(id).ok_or(PresetError::NotFound)?;
        self.file.active = Some(id);
        self.save()?;
        Ok(self.get(id).expect("checked above"))
    }

    /// A new preset from the current settings, made active.
    pub fn create(
        &mut self,
        name: &str,
        gain_db: f32,
        output_device: Option<DeviceRef>,
    ) -> Result<Uuid, PresetError> {
        let name = self.check_name(name, None)?;
        let id = Uuid::new_v4();
        self.file.presets.push(Preset {
            id,
            name,
            gain_db,
            output_device,
        });
        self.file.active = Some(id);
        self.save()?;
        Ok(id)
    }

    /// Overwrites a preset with the current settings ("Save").
    pub fn update(
        &mut self,
        id: Uuid,
        gain_db: f32,
        output_device: Option<DeviceRef>,
    ) -> Result<(), PresetError> {
        let preset = self.get_mut(id)?;
        preset.gain_db = gain_db;
        preset.output_device = output_device;
        self.save()
    }

    pub fn rename(&mut self, id: Uuid, name: &str) -> Result<(), PresetError> {
        let name = self.check_name(name, Some(id))?;
        self.get_mut(id)?.name = name;
        self.save()
    }

    /// A copy named "<name> copy", or "<name> copy 2" and so on if that is
    /// taken. The copy is not made active.
    pub fn duplicate(&mut self, id: Uuid) -> Result<Uuid, PresetError> {
        let original = self.get(id).ok_or(PresetError::NotFound)?.clone();
        let name = (1..)
            .map(|n| match n {
                1 => format!("{} copy", original.name),
                n => format!("{} copy {n}", original.name),
            })
            .map(|name| truncate_name(&name))
            .find(|name| self.check_name(name, None).is_ok())
            .expect("an unbounded search finds a free name");
        let copy = Preset {
            id: Uuid::new_v4(),
            name,
            ..original
        };
        let copy_id = copy.id;
        self.file.presets.push(copy);
        self.save()?;
        Ok(copy_id)
    }

    /// Removes a preset. The last one stays: there is always something to
    /// load. If the active one goes, nothing is active.
    pub fn delete(&mut self, id: Uuid) -> Result<(), PresetError> {
        self.get(id).ok_or(PresetError::NotFound)?;
        if self.file.presets.len() == 1 {
            return Err(PresetError::LastPreset);
        }
        self.file.presets.retain(|p| p.id != id);
        self.drop_dangling_active();
        self.save()
    }

    /// Trims `name` and checks it against the rules. `renaming` is the
    /// preset whose own name does not count as taken.
    pub fn check_name(&self, name: &str, renaming: Option<Uuid>) -> Result<String, PresetError> {
        let name = name.trim();
        let chars = name.chars().count();
        if chars < NAME_LEN_RANGE.0 {
            return Err(PresetError::NameEmpty);
        }
        if chars > NAME_LEN_RANGE.1 {
            return Err(PresetError::NameTooLong);
        }
        let lower = name.to_lowercase();
        if self
            .file
            .presets
            .iter()
            .any(|p| Some(p.id) != renaming && p.name.to_lowercase() == lower)
        {
            return Err(PresetError::NameTaken(name.to_owned()));
        }
        Ok(name.to_owned())
    }

    fn get_mut(&mut self, id: Uuid) -> Result<&mut Preset, PresetError> {
        self.file
            .presets
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or(PresetError::NotFound)
    }

    fn drop_dangling_active(&mut self) {
        if self.file.active.is_some_and(|id| self.get(id).is_none()) {
            self.file.active = None;
        }
    }

    /// Writes the temp file, flushes it, keeps the old file as `.bak`, then
    /// renames the temp over `presets.json`.
    fn save(&self) -> Result<(), PresetError> {
        let Some(path) = &self.path else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let temp = path.with_extension("json.tmp");
        {
            let mut out = fs::File::create(&temp)?;
            out.write_all(serde_json::to_string_pretty(&self.file)?.as_bytes())?;
            out.sync_all()?;
        }
        if path.exists() {
            fs::copy(path, backup_path(path))?;
        }
        fs::rename(&temp, path)?;
        Ok(())
    }
}

fn backup_path(path: &Path) -> PathBuf {
    path.with_extension("json.bak")
}

/// Reads `path`, else its backup, else the built-ins. A file that could not
/// be read is moved aside first, so the save that follows cannot copy it
/// over the good backup.
fn read_with_fallback(path: &Path) -> (PresetFile, LoadOutcome) {
    let main_error = match read(path) {
        Ok(Some(file)) => return (file, LoadOutcome::Loaded),
        // No presets.json. A backup on its own means the last save died
        // between the copy and the rename; it is the newest good copy.
        Ok(None) => match read(&backup_path(path)) {
            Ok(Some(file)) => return (file, LoadOutcome::Loaded),
            _ => return (PresetFile::built_ins(), LoadOutcome::FirstRun),
        },
        Err(error) => error,
    };
    log::warn!("{} is unreadable: {main_error}", path.display());
    let broken = move_aside(path);

    match read(&backup_path(path)) {
        Ok(Some(file)) => {
            log::warn!("Presets restored from the backup");
            match broken {
                Some(broken) => (file, LoadOutcome::FromBackup { broken }),
                None => (file, LoadOutcome::Loaded),
            }
        }
        Ok(None) | Err(_) => {
            log::warn!("No usable backup either; using the built-in presets");
            (PresetFile::built_ins(), LoadOutcome::Reset { broken })
        }
    }
}

/// `Ok(None)` when the file does not exist.
fn read(path: &Path) -> Result<Option<PresetFile>, String> {
    match fs::read_to_string(path) {
        Ok(text) => migrate::parse(&text).map(Some).map_err(|e| e.to_string()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.to_string()),
    }
}

/// Renames an unreadable `presets.json` to `presets.broken-<unix secs>.json`.
fn move_aside(path: &Path) -> Option<PathBuf> {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let broken = path.with_file_name(format!("presets.broken-{stamp}.json"));
    match fs::rename(path, &broken) {
        Ok(()) => Some(broken),
        Err(error) => {
            log::error!("Could not move the broken presets file aside: {error}");
            None
        }
    }
}

/// Cuts a generated name to the length limit, on a character boundary.
fn truncate_name(name: &str) -> String {
    name.chars().take(NAME_LEN_RANGE.1).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(test: &str) -> Self {
            let dir = std::env::temp_dir()
                .join(format!("boostdeck-presets-{test}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        fn file(&self, name: &str) -> PathBuf {
            self.0.join(name)
        }

        fn broken_files(&self) -> Vec<String> {
            fs::read_dir(&self.0)
                .unwrap()
                .filter_map(|e| e.ok()?.file_name().into_string().ok())
                .filter(|n| n.starts_with("presets.broken-"))
                .collect()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn names(store: &PresetStore) -> Vec<&str> {
        store.list().iter().map(|p| p.name.as_str()).collect()
    }

    fn speakers() -> Option<DeviceRef> {
        Some(DeviceRef {
            id: "{0.0.0.00000000}.{speakers}".to_owned(),
            name: "Speakers".to_owned(),
        })
    }

    #[test]
    fn a_first_run_writes_the_built_ins() {
        let dir = TempDir::new("first-run");
        let (store, outcome) = PresetStore::load(Some(&dir.0));
        assert_eq!(outcome, LoadOutcome::FirstRun);
        assert_eq!(names(&store), ["Default", "Max", "Quiet videos"]);
        assert_eq!(store.active().unwrap().name, "Default");
        assert!(dir.file(FILE_NAME).exists());
    }

    /// The README's test: save/load round trip.
    #[test]
    fn everything_survives_a_save_and_reload() {
        let dir = TempDir::new("round-trip");
        let (mut store, _) = PresetStore::load(Some(&dir.0));
        let id = store.create("Movies", 7.5, speakers()).unwrap();

        let (reloaded, outcome) = PresetStore::load(Some(&dir.0));
        assert_eq!(outcome, LoadOutcome::Loaded);
        let movies = reloaded.get(id).unwrap();
        assert_eq!(movies.gain_db, 7.5);
        assert_eq!(movies.output_device, speakers());
        assert_eq!(reloaded.active().unwrap().id, id);
    }

    /// The README's test: name rules.
    #[test]
    fn names_are_trimmed_bounded_and_unique_ignoring_case() {
        let dir = TempDir::new("names");
        let (mut store, _) = PresetStore::load(Some(&dir.0));

        let id = store.create("  Night  ", 0.0, None).unwrap();
        assert_eq!(store.get(id).unwrap().name, "Night");

        assert!(matches!(
            store.create("   ", 0.0, None),
            Err(PresetError::NameEmpty)
        ));
        assert!(matches!(
            store.create(&"x".repeat(41), 0.0, None),
            Err(PresetError::NameTooLong)
        ));
        assert!(store.create(&"x".repeat(40), 0.0, None).is_ok());
        assert!(matches!(
            store.create("quiet VIDEOS", 0.0, None),
            Err(PresetError::NameTaken(_))
        ));
        assert!(matches!(
            store.rename(id, "max"),
            Err(PresetError::NameTaken(_))
        ));
        // A preset may keep its own name, in another case.
        store.rename(id, "NIGHT").unwrap();
        assert_eq!(store.get(id).unwrap().name, "NIGHT");
    }

    #[test]
    fn duplicates_get_numbered_copy_names() {
        let dir = TempDir::new("duplicate");
        let (mut store, _) = PresetStore::load(Some(&dir.0));
        let max = store.list().iter().find(|p| p.name == "Max").unwrap().id;

        let first = store.duplicate(max).unwrap();
        let second = store.duplicate(max).unwrap();
        assert_eq!(store.get(first).unwrap().name, "Max copy");
        assert_eq!(store.get(second).unwrap().name, "Max copy 2");
        assert_eq!(store.get(second).unwrap().gain_db, 12.0);
        assert_ne!(first, second);
    }

    /// The README's test: last-preset protection.
    #[test]
    fn the_last_preset_cannot_be_deleted() {
        let dir = TempDir::new("delete");
        let (mut store, _) = PresetStore::load(Some(&dir.0));
        let ids: Vec<Uuid> = store.list().iter().map(|p| p.id).collect();
        let active = store.active().unwrap().id;

        store.delete(active).unwrap();
        assert_eq!(store.active(), None, "deleting the active one clears it");
        let left: Vec<Uuid> = ids.into_iter().filter(|id| *id != active).collect();
        store.delete(left[0]).unwrap();
        assert!(matches!(
            store.delete(left[1]),
            Err(PresetError::LastPreset)
        ));
        assert_eq!(store.list().len(), 1);
    }

    #[test]
    fn update_overwrites_and_set_active_is_remembered() {
        let dir = TempDir::new("update");
        let (mut store, _) = PresetStore::load(Some(&dir.0));
        let max = store.list().iter().find(|p| p.name == "Max").unwrap().id;
        store.update(max, 9.0, speakers()).unwrap();
        store.set_active(max).unwrap();

        let (reloaded, _) = PresetStore::load(Some(&dir.0));
        assert_eq!(reloaded.get(max).unwrap().gain_db, 9.0);
        assert_eq!(reloaded.active().unwrap().id, max);
    }

    /// The README's test, and step 9's "done when": a corrupt file falls
    /// back to the backup.
    #[test]
    fn a_corrupt_file_is_recovered_from_the_backup() {
        let dir = TempDir::new("corrupt");
        let (mut store, _) = PresetStore::load(Some(&dir.0));
        // Two saves, so the backup holds a full file.
        let id = store.create("Movies", 6.0, None).unwrap();
        store.rename(id, "Films").unwrap();

        fs::write(
            dir.file(FILE_NAME),
            "{ \"schema_version\": 1, \"presets\": [ trunc",
        )
        .unwrap();

        let (recovered, outcome) = PresetStore::load(Some(&dir.0));
        assert!(
            matches!(outcome, LoadOutcome::FromBackup { .. }),
            "{outcome:?}"
        );
        assert!(outcome.notice().is_some());
        // The backup is one save behind: the preset exists, under its old name.
        assert_eq!(recovered.get(id).unwrap().name, "Movies");
        assert_eq!(dir.broken_files().len(), 1);
        // The save that repaired presets.json must not have copied the broken
        // file over the good backup.
        let backup = fs::read_to_string(dir.file("presets.json.bak")).unwrap();
        assert!(
            migrate::parse(&backup).is_ok(),
            "backup clobbered: {backup}"
        );

        // The repaired file reads cleanly next time.
        let (_, outcome) = PresetStore::load(Some(&dir.0));
        assert_eq!(outcome, LoadOutcome::Loaded);
    }

    /// The README's test: ...then to the built-ins.
    #[test]
    fn a_corrupt_file_and_backup_fall_back_to_the_built_ins() {
        let dir = TempDir::new("both-corrupt");
        let (mut store, _) = PresetStore::load(Some(&dir.0));
        store.create("Movies", 6.0, None).unwrap();
        fs::write(dir.file(FILE_NAME), "garbage").unwrap();
        fs::write(dir.file("presets.json.bak"), "more garbage").unwrap();

        let (recovered, outcome) = PresetStore::load(Some(&dir.0));
        assert!(
            matches!(outcome, LoadOutcome::Reset { broken: Some(_) }),
            "{outcome:?}"
        );
        assert_eq!(names(&recovered), ["Default", "Max", "Quiet videos"]);
        assert_eq!(dir.broken_files().len(), 1);
    }

    /// A save that died between keeping the backup and the rename leaves a
    /// backup and no presets.json; that backup is the latest good file.
    #[test]
    fn a_backup_alone_is_used_rather_than_starting_over() {
        let dir = TempDir::new("backup-only");
        let (mut store, _) = PresetStore::load(Some(&dir.0));
        let id = store.create("Movies", 6.0, None).unwrap();
        fs::copy(dir.file(FILE_NAME), dir.file("presets.json.bak")).unwrap();
        fs::remove_file(dir.file(FILE_NAME)).unwrap();

        let (recovered, _) = PresetStore::load(Some(&dir.0));
        assert!(recovered.get(id).is_some());
    }

    #[test]
    fn a_version_0_file_on_disk_is_migrated_and_rewritten() {
        let dir = TempDir::new("v0");
        fs::write(
            dir.file(FILE_NAME),
            r#"[{ "name": "Movies", "gain_db": 6.0, "output_device_id": "x" }]"#,
        )
        .unwrap();
        let (store, outcome) = PresetStore::load(Some(&dir.0));
        assert_eq!(outcome, LoadOutcome::Loaded);
        assert_eq!(names(&store), ["Movies"]);
    }
}
