//! `cargo xtask dist`: the release archive for the platform it runs on, in `dist/`.
//!
//! - Windows: `microcut-<version>-windows-<arch>.zip` with `microcut.exe`
//! - macOS: `microcut-<version>-macos-universal.zip` with `MicroCut.app` (Apple Silicon + Intel in one
//!   binary, ad-hoc signed so Gatekeeper offers "Open Anyway" instead of calling it damaged)
//! - Linux: `microcut-<version>-linux-<arch>.tar.gz` with `microcut` (tar keeps the executable bit)
//!
//! Each archive holds one folder named like the archive, with the README, the licenses and NOTICE.
//! Archives are made with the system's tools (`tar`, `ditto`), so xtask needs no dependencies.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

const APP_NAME: &str = "MicroCut";
const BIN: &str = "microcut";
const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Files from the workspace root that go into every archive.
const DOCS: &[&str] = &["README.md", "LICENSE-MIT", "LICENSE-APACHE", "NOTICE"];
/// The macOS binary is built for both and merged with `lipo`.
const MACOS_TARGETS: &[&str] = &["aarch64-apple-darwin", "x86_64-apple-darwin"];
const BUNDLE_ID: &str = "io.github.najoko.microcut";
const MACOS_MIN_VERSION: &str = "11.0";
/// The app icon (1024 px), relative to the workspace root.
const ICON_PNG: &str = "apps/microcut/assets/icon.png";
/// Pictures of a macOS `.iconset`: file name and edge length in pixels.
const ICONSET: &[(&str, u32)] = &[
    ("icon_16x16.png", 16),
    ("icon_16x16@2x.png", 32),
    ("icon_32x32.png", 32),
    ("icon_32x32@2x.png", 64),
    ("icon_128x128.png", 128),
    ("icon_128x128@2x.png", 256),
    ("icon_256x256.png", 256),
    ("icon_256x256@2x.png", 512),
    ("icon_512x512.png", 512),
    ("icon_512x512@2x.png", 1024),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Platform {
    Windows,
    MacOs,
    Linux,
}

impl Platform {
    fn host() -> Result<Platform, String> {
        match env::consts::OS {
            "windows" => Ok(Platform::Windows),
            "macos" => Ok(Platform::MacOs),
            "linux" => Ok(Platform::Linux),
            other => Err(format!("no release archive is defined for {other}")),
        }
    }

    /// Folder and archive name without extension, e.g. `microcut-0.1.0-windows-x86_64`.
    fn package_name(self, arch: &str) -> String {
        let platform = match self {
            Platform::Windows => format!("windows-{arch}"),
            Platform::MacOs => "macos-universal".to_owned(),
            Platform::Linux => format!("linux-{arch}"),
        };
        format!("{BIN}-{VERSION}-{platform}")
    }

    fn archive_extension(self) -> &'static str {
        match self {
            Platform::Windows | Platform::MacOs => "zip",
            Platform::Linux => "tar.gz",
        }
    }
}

/// Builds the release binary, stages it with the docs and packs the archive; returns its path.
pub fn run(cargo: &str) -> Result<PathBuf, String> {
    let root = workspace_root()?;
    let target_dir = env::var_os("CARGO_TARGET_DIR")
        .map(|dir| root.join(dir))
        .unwrap_or_else(|| root.join("target"));
    let platform = Platform::host()?;
    let name = platform.package_name(env::consts::ARCH);
    let dist = root.join("dist");
    let stage = dist.join(&name);
    let archive = dist.join(format!("{name}.{}", platform.archive_extension()));

    if stage.exists() {
        fs::remove_dir_all(&stage)
            .map_err(|e| format!("could not clear {}: {e}", stage.display()))?;
    }
    if archive.exists() {
        fs::remove_file(&archive)
            .map_err(|e| format!("could not remove {}: {e}", archive.display()))?;
    }
    create_dir(&stage)?;

    match platform {
        Platform::Windows | Platform::Linux => {
            build(cargo, &root, &target_dir, None)?;
            let file = format!("{BIN}{}", env::consts::EXE_SUFFIX);
            copy(&target_dir.join("release").join(&file), &stage.join(&file))?;
        }
        Platform::MacOs => bundle_app(cargo, &root, &target_dir, &stage)?,
    }
    for doc in DOCS {
        copy(&root.join(doc), &stage.join(doc))?;
    }

    match platform {
        Platform::Windows => {
            // The `tar` of Windows (bsdtar) writes zip files; a Git or MSYS `tar` earlier in PATH would not.
            let system_root = env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
            let tar = Path::new(&system_root).join("System32").join("tar.exe");
            let mut cmd = Command::new(tar);
            cmd.arg("-a").arg("-c").arg("-f").arg(&archive);
            cmd.arg("-C").arg(&dist).arg(&name);
            run_tool(&mut cmd, "tar")?;
        }
        Platform::MacOs => {
            let mut cmd = Command::new("ditto");
            cmd.args(["-c", "-k", "--sequesterRsrc", "--keepParent"]);
            cmd.arg(&stage).arg(&archive);
            run_tool(&mut cmd, "ditto")?;
        }
        Platform::Linux => {
            let mut cmd = Command::new("tar");
            cmd.arg("-czf")
                .arg(&archive)
                .arg("-C")
                .arg(&dist)
                .arg(&name);
            run_tool(&mut cmd, "tar")?;
        }
    }
    fs::remove_dir_all(&stage)
        .map_err(|e| format!("could not clean up {}: {e}", stage.display()))?;
    Ok(archive)
}

/// `MicroCut.app` in `stage`: universal binary, `Info.plist`, ad-hoc signature.
fn bundle_app(cargo: &str, root: &Path, target_dir: &Path, stage: &Path) -> Result<(), String> {
    let app = stage.join(format!("{APP_NAME}.app"));
    let contents = app.join("Contents");
    let macos = contents.join("MacOS");
    let resources = contents.join("Resources");
    create_dir(&macos)?;
    create_dir(&resources)?;
    icns(
        &root.join(ICON_PNG),
        &resources.join(format!("{APP_NAME}.icns")),
    )?;

    let mut lipo = Command::new("lipo");
    lipo.arg("-create").arg("-output").arg(macos.join(BIN));
    for target in MACOS_TARGETS {
        build(cargo, root, target_dir, Some(target))?;
        lipo.arg(target_dir.join(target).join("release").join(BIN));
    }
    run_tool(&mut lipo, "lipo")?;

    write(&contents.join("Info.plist"), &info_plist())?;
    write(&contents.join("PkgInfo"), "APPL????")?;

    let mut sign = Command::new("codesign");
    sign.args(["--force", "--deep", "--sign", "-"]).arg(&app);
    run_tool(&mut sign, "codesign")
}

/// `icon.png` scaled to every size of an `.iconset` (`sips`), packed into `.icns` (`iconutil`).
fn icns(png: &Path, icns: &Path) -> Result<(), String> {
    let iconset = icns.with_extension("iconset");
    create_dir(&iconset)?;
    for (file, edge) in ICONSET {
        let edge = edge.to_string();
        let mut sips = Command::new("sips");
        sips.args(["-z", &edge, &edge]).arg(png);
        sips.arg("--out").arg(iconset.join(file));
        sips.stdout(std::process::Stdio::null());
        run_tool(&mut sips, "sips")?;
    }
    let mut iconutil = Command::new("iconutil");
    iconutil
        .args(["-c", "icns"])
        .arg(&iconset)
        .arg("-o")
        .arg(icns);
    run_tool(&mut iconutil, "iconutil")?;
    fs::remove_dir_all(&iconset)
        .map_err(|e| format!("could not clean up {}: {e}", iconset.display()))
}

/// The bundle's `Info.plist`.
fn info_plist() -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>{APP_NAME}</string>
	<key>CFBundleDisplayName</key>
	<string>{APP_NAME}</string>
	<key>CFBundleIdentifier</key>
	<string>{BUNDLE_ID}</string>
	<key>CFBundleExecutable</key>
	<string>{BIN}</string>
	<key>CFBundleIconFile</key>
	<string>{APP_NAME}</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>{VERSION}</string>
	<key>CFBundleVersion</key>
	<string>{VERSION}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>LSMinimumSystemVersion</key>
	<string>{MACOS_MIN_VERSION}</string>
	<key>LSApplicationCategoryType</key>
	<string>public.app-category.video</string>
	<key>NSHighResolutionCapable</key>
	<true/>
</dict>
</plist>
"#
    )
}

/// `cargo build --release --locked -p microcut`, optionally for another target.
fn build(cargo: &str, root: &Path, target_dir: &Path, target: Option<&str>) -> Result<(), String> {
    let mut cmd = Command::new(cargo);
    cmd.current_dir(root);
    cmd.args(["build", "--release", "--locked", "--package", BIN]);
    cmd.arg("--target-dir").arg(target_dir);
    if let Some(target) = target {
        cmd.args(["--target", target]);
    }
    if cfg!(target_os = "macos") {
        // The oldest macOS the bundle claims to run on; also passed to the linker.
        cmd.env("MACOSX_DEPLOYMENT_TARGET", MACOS_MIN_VERSION);
    }
    eprintln!("xtask: building {BIN} ({})", target.unwrap_or("host"));
    run_tool(&mut cmd, "cargo build")
}

fn workspace_root() -> Result<PathBuf, String> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask is not inside the workspace".to_owned())
}

fn run_tool(cmd: &mut Command, name: &str) -> Result<(), String> {
    match cmd.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("{name} failed ({status})")),
        Err(e) => Err(format!("could not run {name}: {e}")),
    }
}

fn create_dir(dir: &Path) -> Result<(), String> {
    fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))
}

fn copy(from: &Path, to: &Path) -> Result<(), String> {
    fs::copy(from, to)
        .map(drop)
        .map_err(|e| format!("could not copy {} to {}: {e}", from.display(), to.display()))
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    fs::write(path, text).map_err(|e| format!("could not write {}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn archive_names() {
        let v = VERSION;
        assert_eq!(
            Platform::Windows.package_name("x86_64"),
            format!("microcut-{v}-windows-x86_64")
        );
        assert_eq!(
            Platform::MacOs.package_name("aarch64"),
            format!("microcut-{v}-macos-universal")
        );
        assert_eq!(
            Platform::Linux.package_name("x86_64"),
            format!("microcut-{v}-linux-x86_64")
        );
        assert_eq!(Platform::Linux.archive_extension(), "tar.gz");
        assert_eq!(Platform::Windows.archive_extension(), "zip");
    }

    #[test]
    fn info_plist_names_the_binary_and_version() {
        let plist = info_plist();
        assert!(plist.contains(&format!(
            "<key>CFBundleExecutable</key>\n\t<string>{BIN}</string>"
        )));
        assert!(plist.contains(&format!(
            "<key>CFBundleShortVersionString</key>\n\t<string>{VERSION}</string>"
        )));
        assert!(plist.contains(BUNDLE_ID));
        // Every opened element is closed.
        assert_eq!(
            plist.matches("<key>").count(),
            plist.matches("</key>").count()
        );
        assert_eq!(
            plist.matches("<string>").count(),
            plist.matches("</string>").count()
        );
        assert!(plist.trim_end().ends_with("</plist>"));
    }

    #[test]
    fn the_packed_files_exist() {
        let root = workspace_root().unwrap();
        assert!(root.join(ICON_PNG).is_file(), "{ICON_PNG} is missing");
        for doc in DOCS {
            assert!(root.join(doc).is_file(), "{doc} is missing");
        }
    }
}
