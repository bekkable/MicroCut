//! The version of the installed `ffmpeg`, and what follows from it.

use std::fmt;

/// `major.minor` of an FFmpeg release.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct FfmpegVersion {
    /// e.g. 4 in 4.4.2
    pub major: u32,
    /// e.g. 4 in 4.4.2
    pub minor: u32,
}

impl FfmpegVersion {
    /// The oldest FFmpeg MicroCut works with (`-stats_period` for the export progress, NVENC
    /// presets `p1`–`p7`); Ubuntu 22.04 ships 4.4.
    pub const MINIMUM: FfmpegVersion = FfmpegVersion { major: 4, minor: 4 };

    /// From here on `-fps_mode` replaces the deprecated `-vsync`.
    const FPS_MODE: FfmpegVersion = FfmpegVersion { major: 5, minor: 1 };

    /// The version in the first line of `ffmpeg -version`, e.g. `ffmpeg version 4.4.2-0ubuntu0.22.04.1
    /// Copyright …`, `ffmpeg version n7.1.1-6-g48c0f071d4-20250405` or `ffmpeg version 6.0-full_build-www.gyan.dev`.
    /// `None` for builds without a release number (`N-118896-g…`, `2024-03-14-git-…`): those are built from
    /// the development branch and count as new.
    pub fn parse(output: &str) -> Option<FfmpegVersion> {
        let first = output.lines().next()?;
        let token = first
            .trim()
            .strip_prefix("ffmpeg version ")?
            .split_whitespace()
            .next()?;
        let numbers = token.strip_prefix('n').unwrap_or(token);
        let release: &str = numbers
            .split(|c: char| !(c.is_ascii_digit() || c == '.'))
            .next()?;
        let mut parts = release.split('.');
        let major: u32 = parts.next()?.parse().ok()?;
        let minor: u32 = match parts.next() {
            Some(minor) => minor.parse().ok()?,
            None => 0,
        };
        // A date (`2024-03-14-git-…`), not a release.
        if major >= 100 {
            return None;
        }
        Some(FfmpegVersion { major, minor })
    }

    /// The option that passes every decoded picture through unchanged (no duplicates, no drops).
    pub(crate) fn passthrough(version: Option<FfmpegVersion>) -> [&'static str; 2] {
        match version {
            Some(v) if v < Self::FPS_MODE => ["-vsync", "passthrough"],
            _ => ["-fps_mode", "passthrough"],
        }
    }
}

impl fmt::Display for FfmpegVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.major, self.minor)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(major: u32, minor: u32) -> Option<FfmpegVersion> {
        Some(FfmpegVersion { major, minor })
    }

    #[test]
    fn versions_of_real_builds() {
        let parse = FfmpegVersion::parse;
        // Ubuntu 22.04, Debian 12, Homebrew, gyan.dev, BtbN release and git builds.
        assert_eq!(
            parse(
                "ffmpeg version 4.4.2-0ubuntu0.22.04.1 Copyright (c) 2000-2021 the FFmpeg developers\nbuilt with gcc"
            ),
            v(4, 4)
        );
        assert_eq!(parse("ffmpeg version 5.1.6-0+deb12u1 Copyright"), v(5, 1));
        assert_eq!(
            parse("ffmpeg version 7.1.1 Copyright (c) 2000-2025"),
            v(7, 1)
        );
        assert_eq!(
            parse("ffmpeg version 6.0-full_build-www.gyan.dev Copyright"),
            v(6, 0)
        );
        assert_eq!(
            parse("ffmpeg version n7.1.1-6-g48c0f071d4-20250405 Copyright"),
            v(7, 1)
        );
        assert_eq!(parse("ffmpeg version 5 Copyright"), v(5, 0));
        assert_eq!(
            parse("ffmpeg version N-118896-g9f0970ee35-20250327 Copyright"),
            None
        );
        assert_eq!(
            parse("ffmpeg version 2024-03-14-git-1a2b3c Copyright"),
            None
        );
        // Not ffmpeg's output at all.
        assert_eq!(parse(""), None);
        assert_eq!(parse("ffprobe version 6.0"), None);
        assert_eq!(parse("ffmpeg version"), None);
        assert_eq!(parse("ffmpeg version 99999999999999999999.1"), None);
    }

    #[test]
    fn passthrough_option_by_version() {
        let pass = FfmpegVersion::passthrough;
        assert_eq!(pass(v(4, 4)), ["-vsync", "passthrough"]);
        assert_eq!(pass(v(5, 0)), ["-vsync", "passthrough"]);
        assert_eq!(pass(v(5, 1)), ["-fps_mode", "passthrough"]);
        assert_eq!(pass(v(8, 0)), ["-fps_mode", "passthrough"]);
        assert_eq!(pass(None), ["-fps_mode", "passthrough"]);
    }

    #[test]
    fn minimum() {
        assert!(FfmpegVersion { major: 4, minor: 3 } < FfmpegVersion::MINIMUM);
        assert!(FfmpegVersion { major: 4, minor: 4 } >= FfmpegVersion::MINIMUM);
        assert!(FfmpegVersion { major: 5, minor: 0 } >= FfmpegVersion::MINIMUM);
        assert_eq!(FfmpegVersion::MINIMUM.to_string(), "4.4");
    }
}
