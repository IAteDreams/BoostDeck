//! Schema migrations.
//!
//! A `presets.json` written by an older `schema_version` is upgraded one step
//! at a time, as JSON, until it matches `SCHEMA_VERSION`, and only then
//! parsed into the current structs. A file with no `schema_version` at all
//! is version 0.
//!
//! Version 0 is the pre-release layout: a bare array of presets with no ids,
//! the output as a plain `output_device_id` string, and no active preset.
//!
//! ```json
//! [{ "name": "Movies", "gain_db": 6.0, "output_device_id": "{0.0.0.00000000}.{...}" }]
//! ```

use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::SCHEMA_VERSION;
use crate::model::PresetFile;

#[derive(Debug, thiserror::Error)]
pub enum MigrateError {
    #[error("not JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("schema version {0} is newer than this BoostDeck understands ({SCHEMA_VERSION})")]
    TooNew(u64),
    #[error("not a presets file: {0}")]
    Shape(String),
}

/// Parses `text` as a presets file of any known version.
pub fn parse(text: &str) -> Result<PresetFile, MigrateError> {
    let mut value: Value = serde_json::from_str(text)?;
    loop {
        match version_of(&value)? {
            0 => value = v0_to_v1(value)?,
            v if v == u64::from(SCHEMA_VERSION) => break,
            v => return Err(MigrateError::TooNew(v)),
        }
    }
    let file: PresetFile = serde_json::from_value(value)?;
    if file.presets.is_empty() {
        return Err(MigrateError::Shape("no presets in it".to_owned()));
    }
    Ok(file)
}

fn version_of(value: &Value) -> Result<u64, MigrateError> {
    match value {
        Value::Array(_) => Ok(0),
        Value::Object(map) => match map.get("schema_version") {
            None => Ok(0),
            Some(v) => v
                .as_u64()
                .ok_or_else(|| MigrateError::Shape("schema_version is not a number".to_owned())),
        },
        _ => Err(MigrateError::Shape(
            "neither an object nor an array".to_owned(),
        )),
    }
}

/// Wraps the bare array, gives every preset an id, and turns the plain
/// device id into a device reference. The name is unknown in version 0, so
/// it is left empty until the device is next seen.
fn v0_to_v1(value: Value) -> Result<Value, MigrateError> {
    let Value::Array(presets) = value else {
        return Err(MigrateError::Shape("version 0 must be an array".to_owned()));
    };
    let presets = presets
        .into_iter()
        .map(|preset| {
            let Value::Object(mut preset) = preset else {
                return Err(MigrateError::Shape("a preset is not an object".to_owned()));
            };
            let mut upgraded = Map::new();
            upgraded.insert("id".to_owned(), json!(Uuid::new_v4()));
            upgraded.insert(
                "name".to_owned(),
                preset.remove("name").unwrap_or(json!("")),
            );
            upgraded.insert(
                "gain_db".to_owned(),
                preset.remove("gain_db").unwrap_or(json!(0.0)),
            );
            let device = match preset.remove("output_device_id") {
                Some(Value::String(id)) => json!({ "id": id, "name": "" }),
                _ => Value::Null,
            };
            upgraded.insert("output_device".to_owned(), device);
            Ok(Value::Object(upgraded))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(json!({ "schema_version": 1, "active": null, "presets": presets }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The README's test: a version 0 fixture migrates to version 1.
    #[test]
    fn a_version_0_file_migrates_to_version_1() {
        let v0 = r#"[
            { "name": "Movies", "gain_db": 6.0, "output_device_id": "{0.0.0.00000000}.{abc}" },
            { "name": "Night", "gain_db": -6.0 }
        ]"#;
        let file = parse(v0).unwrap();
        assert_eq!(file.schema_version, SCHEMA_VERSION);
        assert_eq!(file.active, None);
        assert_eq!(file.presets.len(), 2);

        let movies = &file.presets[0];
        assert_eq!(movies.name, "Movies");
        assert_eq!(movies.gain_db, 6.0);
        let device = movies.output_device.as_ref().unwrap();
        assert_eq!(device.id, "{0.0.0.00000000}.{abc}");

        assert_eq!(file.presets[1].output_device, None);
        assert_ne!(file.presets[0].id, file.presets[1].id, "ids must be unique");
    }

    #[test]
    fn a_current_file_parses_as_it_is() {
        let file = PresetFile::built_ins();
        let text = serde_json::to_string(&file).unwrap();
        assert_eq!(parse(&text).unwrap(), file);
    }

    #[test]
    fn a_newer_file_is_refused_not_mangled() {
        let text = r#"{ "schema_version": 99, "presets": [] }"#;
        assert!(matches!(parse(text), Err(MigrateError::TooNew(99))));
    }

    #[test]
    fn garbage_and_empty_files_are_refused() {
        for text in [
            "{ not json",
            "42",
            r#"{ "schema_version": 1, "presets": [] }"#,
            "",
        ] {
            assert!(parse(text).is_err(), "{text:?} parsed");
        }
    }
}
