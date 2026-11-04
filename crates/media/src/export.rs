//! What an export asks for: the frame range, codecs, quality and audio track.

use std::path::PathBuf;

/// Video codec of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoCodec {
    /// H.264 / AVC (`libx264`): plays everywhere.
    H264,
    /// H.265 / HEVC (`libx265`): smaller, slower to encode, not every player has it.
    H265,
    /// AV1 (`libsvtav1`): smallest, newest players only.
    Av1,
}

impl VideoCodec {
    /// All codecs, in the order the export dialog offers them.
    pub const ALL: [VideoCodec; 3] = [VideoCodec::H264, VideoCodec::H265, VideoCodec::Av1];

    /// Name for the user.
    pub fn label(self) -> &'static str {
        match self {
            VideoCodec::H264 => "H.264",
            VideoCodec::H265 => "H.265 (HEVC)",
            VideoCodec::Av1 => "AV1",
        }
    }

    /// The ffmpeg encoder that produces it.
    pub fn encoder(self) -> &'static str {
        match self {
            VideoCodec::H264 => "libx264",
            VideoCodec::H265 => "libx265",
            VideoCodec::Av1 => "libsvtav1",
        }
    }

    /// The NVIDIA encoder (NVENC) for hardware-accelerated exports.
    pub fn hardware_encoder(self) -> &'static str {
        match self {
            VideoCodec::H264 => "h264_nvenc",
            VideoCodec::H265 => "hevc_nvenc",
            VideoCodec::Av1 => "av1_nvenc",
        }
    }

    /// Constant-quality value (`-cq`, preset p5) of the NVIDIA encoder for `quality`: the value
    /// at which NVENC reaches the same VMAF as the software encoder at [`VideoCodec::crf`].
    /// Measured on 720p60 screen recordings (one menu, three gameplay scenes, 10 s each):
    /// x264 CRF 18/23/28 ≙ h264_nvenc CQ 26–30/30–34/35–38, x265 CRF 20/25/30 ≙ hevc_nvenc CQ
    /// 28–31/32–36/37–41; the values lean to the better quality. Files come out about as large
    /// as with the software encoder. AV1 is unmeasured (no av1_nvenc on the test machine) and
    /// follows the H.265 values.
    pub fn hardware_cq(self, quality: VideoQuality) -> u32 {
        match (self, quality) {
            (VideoCodec::H264, VideoQuality::High) => 27,
            (VideoCodec::H264, VideoQuality::Medium) => 31,
            (VideoCodec::H264, VideoQuality::Small) => 36,
            (VideoCodec::H265 | VideoCodec::Av1, VideoQuality::High) => 28,
            (VideoCodec::H265 | VideoCodec::Av1, VideoQuality::Medium) => 33,
            (VideoCodec::H265 | VideoCodec::Av1, VideoQuality::Small) => 38,
        }
    }

    /// Constant-quality value (`-crf`) for `quality`; lower is better and larger.
    pub fn crf(self, quality: VideoQuality) -> u32 {
        match (self, quality) {
            (VideoCodec::H264, VideoQuality::High) => 18,
            (VideoCodec::H264, VideoQuality::Medium) => 23,
            (VideoCodec::H264, VideoQuality::Small) => 28,
            (VideoCodec::H265, VideoQuality::High) => 20,
            (VideoCodec::H265, VideoQuality::Medium) => 25,
            (VideoCodec::H265, VideoQuality::Small) => 30,
            (VideoCodec::Av1, VideoQuality::High) => 24,
            (VideoCodec::Av1, VideoQuality::Medium) => 32,
            (VideoCodec::Av1, VideoQuality::Small) => 40,
        }
    }
}

/// Quality level of the video; each codec maps it to its own constant-quality value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum VideoQuality {
    /// Visually lossless for most material.
    High,
    /// Good quality, about half the size of `High`.
    Medium,
    /// Visible loss, small files.
    Small,
}

impl VideoQuality {
    /// All levels, best first.
    pub const ALL: [VideoQuality; 3] = [
        VideoQuality::High,
        VideoQuality::Medium,
        VideoQuality::Small,
    ];

    /// Name for the user.
    pub fn label(self) -> &'static str {
        match self {
            VideoQuality::High => "High",
            VideoQuality::Medium => "Medium",
            VideoQuality::Small => "Small",
        }
    }
}

/// Audio codec of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AudioCodec {
    /// AAC (ffmpeg's built-in `aac`): plays everywhere.
    Aac,
    /// Opus (`libopus`): better at low bit rates, newer players only.
    Opus,
}

impl AudioCodec {
    /// All codecs, in the order the export dialog offers them.
    pub const ALL: [AudioCodec; 2] = [AudioCodec::Aac, AudioCodec::Opus];

    /// Name for the user.
    pub fn label(self) -> &'static str {
        match self {
            AudioCodec::Aac => "AAC",
            AudioCodec::Opus => "Opus",
        }
    }

    /// The ffmpeg encoder that produces it.
    pub fn encoder(self) -> &'static str {
        match self {
            AudioCodec::Aac => "aac",
            AudioCodec::Opus => "libopus",
        }
    }

    /// Bit rates offered, in kbit/s, highest first.
    pub fn bitrates(self) -> &'static [u32] {
        match self {
            AudioCodec::Aac => &[256, 192, 128, 96],
            AudioCodec::Opus => &[192, 128, 96, 64],
        }
    }

    /// The bit rate chosen by default, in kbit/s.
    pub fn default_bitrate(self) -> u32 {
        match self {
            AudioCodec::Aac => 192,
            AudioCodec::Opus => 128,
        }
    }
}

/// The audio part of an export.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AudioExport {
    /// Index into [`crate::MediaInfo::audio`].
    pub track: usize,
    /// Codec of the exported track.
    pub codec: AudioCodec,
    /// Bit rate in kbit/s (8 to 1024).
    pub kbps: u32,
}

/// One export: frames `from .. from + count` of the media into `output` (MP4).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportJob {
    /// The file to create; must not exist yet.
    pub output: PathBuf,
    /// First frame.
    pub from: i64,
    /// Number of frames (at least 1).
    pub count: i64,
    /// Video codec.
    pub video: VideoCodec,
    /// Video quality.
    pub quality: VideoQuality,
    /// Decode on the GPU (CUDA) and encode with NVENC; see [`Encoders::hardware`].
    pub hardware: bool,
    /// The audio track to include, or `None` for a silent video.
    pub audio: Option<AudioExport>,
}

/// How an export ended without an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportEnd {
    /// The file is complete.
    Finished,
    /// The user cancelled; the partial file was removed.
    Cancelled,
}

/// The encoders the installed FFmpeg offers, among those MicroCut can use.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Encoders {
    /// Available video codecs, in [`VideoCodec::ALL`] order.
    pub video: Vec<VideoCodec>,
    /// Available audio codecs, in [`AudioCodec::ALL`] order.
    pub audio: Vec<AudioCodec>,
    /// Video codecs whose NVIDIA encoder FFmpeg has. Whether a suitable graphics card is
    /// present only shows when exporting.
    pub hardware: Vec<VideoCodec>,
}

impl Encoders {
    /// Picks the usable encoders from the output of `ffmpeg -encoders`, whose lines look like
    /// ` V....D libx264   libx264 H.264 / AVC …`.
    pub(crate) fn parse(listing: &str) -> Encoders {
        let names: Vec<&str> = listing
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace();
                let flags = words.next()?;
                let name = words.next()?;
                // Flags are six characters, the first is the media type (V, A, S).
                (flags.len() == 6 && flags.starts_with(['V', 'A'])).then_some(name)
            })
            .collect();
        let has = |encoder: &str| names.contains(&encoder);
        Encoders {
            video: VideoCodec::ALL
                .into_iter()
                .filter(|c| has(c.encoder()))
                .collect(),
            audio: AudioCodec::ALL
                .into_iter()
                .filter(|c| has(c.encoder()))
                .collect(),
            hardware: VideoCodec::ALL
                .into_iter()
                .filter(|c| has(c.hardware_encoder()))
                .collect(),
        }
    }
}

/// Reads `frame=N` lines of `ffmpeg -progress` output; split across reads is fine.
#[derive(Debug, Default)]
pub(crate) struct ProgressParser {
    line: Vec<u8>,
}

impl ProgressParser {
    /// Feeds output bytes; returns the last frame count they completed, if any.
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> Option<i64> {
        /// Longer lines are not progress keys; they are dropped (bounded memory).
        const MAX_LINE: usize = 256;
        let mut frames = None;
        for &byte in bytes {
            if byte == b'\n' {
                let line = String::from_utf8_lossy(&self.line);
                if let Some(value) = line.trim().strip_prefix("frame=")
                    && let Ok(n) = value.trim().parse::<i64>()
                    && n >= 0
                {
                    frames = Some(n);
                }
                self.line.clear();
            } else if self.line.len() < MAX_LINE {
                self.line.push(byte);
            }
        }
        frames
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encoders_are_read_from_the_listing() {
        let listing = "Encoders:\n V..... = Video\n ------\n \
            V....D libx264              libx264 H.264 / AVC\n \
            V....D libx265rgb           not the one\n \
            V..... libsvtav1            SVT-AV1\n \
            V....D hevc_nvenc           NVIDIA NVENC hevc encoder\n \
            A....D aac                  AAC (Advanced Audio Coding)\n \
            A....D aac_mf               AAC via MediaFoundation\n";
        let encoders = Encoders::parse(listing);
        assert_eq!(encoders.video, [VideoCodec::H264, VideoCodec::Av1]);
        assert_eq!(encoders.audio, [AudioCodec::Aac]);
        assert_eq!(encoders.hardware, [VideoCodec::H265]);
        assert_eq!(Encoders::parse(""), Encoders::default());
    }

    #[test]
    fn progress_lines_give_the_frame_count() {
        let mut parser = ProgressParser::default();
        assert_eq!(parser.feed(b"frame=12\nfps=30.0\nfra"), Some(12));
        assert_eq!(parser.feed(b"me=40\r\nprogress=continue\n"), Some(40));
        assert_eq!(parser.feed(b"frame=-3\nframe=x\n"), None);
        assert_eq!(parser.feed(b"frame=7"), None, "line not complete yet");
        assert_eq!(parser.feed(b"\n"), Some(7));
        let long = vec![b'a'; 10_000];
        assert_eq!(parser.feed(&long), None);
        assert_eq!(parser.feed(b"\nframe=9\n"), Some(9));
    }

    #[test]
    fn every_codec_has_settings() {
        for codec in VideoCodec::ALL {
            let crfs: Vec<_> = VideoQuality::ALL.iter().map(|q| codec.crf(*q)).collect();
            assert!(crfs.is_sorted(), "{codec:?}: better quality, lower crf");
        }
        for codec in AudioCodec::ALL {
            assert!(codec.bitrates().contains(&codec.default_bitrate()));
        }
    }
}
