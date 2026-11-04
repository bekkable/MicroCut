//! Finding the `ffmpeg` and `ffprobe` executables.

use std::env;
use std::path::{Path, PathBuf};

/// Environment variable naming a folder that contains `ffmpeg` and `ffprobe`. Searched first.
pub const FFMPEG_DIR_ENV: &str = "MICROCUT_FFMPEG_DIR";

/// Where package managers put programs on macOS: Homebrew (Apple Silicon, Intel), MacPorts. An app
/// started from the Finder only gets `/usr/bin:/bin:/usr/sbin:/sbin` as `PATH`, so FFmpeg installed
/// by `brew install ffmpeg` would not be found otherwise.
const MACOS_PACKAGE_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"];

/// Folders to search, in order: [`FFMPEG_DIR_ENV`], the folder of the MicroCut executable,
/// every entry of `PATH`, then (on macOS) the package managers' folders.
pub(crate) fn search_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(dir) = env::var_os(FFMPEG_DIR_ENV).filter(|d| !d.is_empty()) {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(dir) = env::current_exe().ok().as_deref().and_then(Path::parent) {
        dirs.push(dir.to_path_buf());
    }
    if let Some(path) = env::var_os("PATH") {
        dirs.extend(env::split_paths(&path).filter(|d| !d.as_os_str().is_empty()));
    }
    if cfg!(target_os = "macos") {
        dirs.extend(MACOS_PACKAGE_DIRS.iter().map(PathBuf::from));
    }
    dirs
}

/// First `dir/<name><exe suffix>` that is a file.
pub(crate) fn find_tool(name: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    let file = format!("{name}{}", env::consts::EXE_SUFFIX);
    dirs.iter().map(|dir| dir.join(&file)).find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn finds_tool_in_first_matching_dir() {
        let base = env::temp_dir().join(format!("microcut-locate-{}", std::process::id()));
        let (a, b, c) = (base.join("a"), base.join("b"), base.join("c"));
        for dir in [&a, &b, &c] {
            fs::create_dir_all(dir).unwrap();
        }
        let file = format!("ffprobe{}", env::consts::EXE_SUFFIX);
        fs::write(b.join(&file), b"").unwrap();
        fs::write(c.join(&file), b"").unwrap();
        // A folder with the tool's name must not count as the tool.
        fs::create_dir_all(a.join(&file)).unwrap();

        let found = find_tool("ffprobe", &[a.clone(), b.clone(), c.clone()]);
        assert_eq!(found, Some(b.join(&file)));
        assert_eq!(find_tool("ffmpeg", &[a, b, c]), None);

        fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn missing_dirs_are_skipped() {
        let nowhere = PathBuf::from("this/folder/does/not/exist");
        assert_eq!(find_tool("ffmpeg", &[nowhere]), None);
        assert_eq!(find_tool("ffmpeg", &[]), None);
    }
}
