//! Settings the user edits by hand, in `config.json` next to the app's saved state
//! (`%APPDATA%\microcut\data`, `~/Library/Application Support/microcut`,
//! `~/.local/share/microcut`). The file is written with the defaults when it is missing; a
//! value that is missing, of the wrong type or out of range falls back to its default, and a
//! file that cannot be read at all leaves every default in place (and is not overwritten).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// File name in the app's data folder.
const FILE_NAME: &str = "config.json";

/// Most frames the buffer may keep to either side of the cursor.
const MAX_BUFFER_FRAMES: i64 = 500;
/// Most memory the buffer may use, in megabytes.
const MAX_BUFFER_MEGABYTES: u64 = 65_536;

/// Everything in `config.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Config {
    /// Decoded frames around the paused cursor.
    pub frame_buffer: FrameBufferConfig,
}

/// The frame buffer (see `framebuffer.rs`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FrameBufferConfig {
    /// Frames kept decoded before and after the cursor while paused (0 turns the buffer off).
    pub frames_each_way: i64,
    /// Upper limit for the buffer's memory; with large pictures it keeps fewer frames.
    pub max_megabytes: u64,
}

impl Default for FrameBufferConfig {
    fn default() -> FrameBufferConfig {
        FrameBufferConfig {
            frames_each_way: 32,
            max_megabytes: 1024,
        }
    }
}

impl Config {
    /// Reads the config from the app's data folder (writing the defaults there first if there
    /// is none). Also returns the file's path, if there is a data folder, and a note if the
    /// file could not be used.
    pub fn load() -> (Config, Option<PathBuf>, Option<String>) {
        let Some(path) = eframe::storage_dir("microcut").map(|dir| dir.join(FILE_NAME)) else {
            return (Config::default(), None, None);
        };
        let (config, note) = Config::load_from(&path);
        (config, Some(path), note)
    }

    fn load_from(path: &Path) -> (Config, Option<String>) {
        match std::fs::read_to_string(path) {
            Ok(text) => match Config::parse(&text) {
                Ok(config) => (config, None),
                Err(err) => (
                    Config::default(),
                    Some(format!(
                        "{} is not valid JSON ({err}); using the defaults.",
                        path.display()
                    )),
                ),
            },
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let config = Config::default();
                // Best effort: without the file the defaults still apply.
                let _ = write_defaults(path, &config);
                (config, None)
            }
            Err(err) => (
                Config::default(),
                Some(format!("Could not read {}: {err}", path.display())),
            ),
        }
    }

    /// Parses the file's text; values out of range are brought into range, values of the wrong
    /// type fall back to their defaults.
    fn parse(text: &str) -> Result<Config, serde_json::Error> {
        let value: serde_json::Value = serde_json::from_str(text)?;
        let mut config = Config::default();
        if let Some(buffer) = value.get("frame_buffer") {
            let defaults = FrameBufferConfig::default();
            config.frame_buffer = FrameBufferConfig {
                frames_each_way: buffer
                    .get("frames_each_way")
                    .and_then(serde_json::Value::as_i64)
                    .map_or(defaults.frames_each_way, |n| n.clamp(0, MAX_BUFFER_FRAMES)),
                max_megabytes: buffer
                    .get("max_megabytes")
                    .and_then(serde_json::Value::as_u64)
                    .map_or(defaults.max_megabytes, |n| n.min(MAX_BUFFER_MEGABYTES)),
            };
        }
        Ok(config)
    }
}

fn write_defaults(path: &Path, config: &Config) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let text = serde_json::to_string_pretty(config).map_err(std::io::Error::other)?;
    // `create_new`: never overwrite a file that appeared meanwhile.
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    file.write_all(b"\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_32_frames_each_way() {
        let config = Config::default();
        assert_eq!(config.frame_buffer.frames_each_way, 32);
        assert_eq!(config.frame_buffer.max_megabytes, 1024);
    }

    #[test]
    fn the_file_is_read_leniently() {
        let parse = |text: &str| Config::parse(text).unwrap().frame_buffer;
        let custom =
            parse(r#"{ "frame_buffer": { "frames_each_way": 64, "max_megabytes": 256 } }"#);
        assert_eq!((custom.frames_each_way, custom.max_megabytes), (64, 256));
        // Missing parts, wrong types, unknown keys: defaults.
        assert_eq!(parse("{}"), FrameBufferConfig::default());
        assert_eq!(
            parse(r#"{ "frame_buffer": { "frames_each_way": "many" }, "other": 1 }"#),
            FrameBufferConfig::default()
        );
        // Out of range: brought into range.
        let wild = parse(r#"{ "frame_buffer": { "frames_each_way": -5, "max_megabytes": 1e30 } }"#);
        assert_eq!(wild.frames_each_way, 0);
        assert_eq!(wild.max_megabytes, 1024, "1e30 is no integer: default");
        let big = parse(r#"{ "frame_buffer": { "frames_each_way": 99999 } }"#);
        assert_eq!(big.frames_each_way, MAX_BUFFER_FRAMES);
        // Not JSON at all.
        assert!(Config::parse("frames = 32").is_err());
    }

    #[test]
    fn a_missing_file_is_written_with_the_defaults_and_read_back() {
        let dir = std::env::temp_dir().join(format!("microcut-config-{}", std::process::id()));
        let path = dir.join(FILE_NAME);
        let _ = std::fs::remove_file(&path);
        let (config, note) = Config::load_from(&path);
        assert_eq!((config, note), (Config::default(), None));
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"frames_each_way\": 32"), "{text}");
        // A broken file is reported, used as defaults and left alone.
        std::fs::write(&path, "{ broken").unwrap();
        let (config, note) = Config::load_from(&path);
        assert_eq!(config, Config::default());
        assert!(note.unwrap().contains("not valid JSON"));
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "{ broken");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
