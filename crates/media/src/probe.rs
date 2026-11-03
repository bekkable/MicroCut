//! Parsing `ffprobe -print_format json -show_format -show_streams` output.
//!
//! Every field is optional and every number is checked: the JSON describes an untrusted file.

use std::path::PathBuf;

use microcut_time::{FrameRate, Tick};
use serde::Deserialize;

use crate::{AudioInfo, MediaError, MediaInfo, VideoInfo};

/// Frame rates above this are treated as bogus (e.g. a 90 kHz timebase reported as rate).
const MAX_FPS: u32 = 1_000;

#[derive(Deserialize, Default)]
#[serde(default)]
struct ProbeJson {
    streams: Vec<StreamJson>,
    format: FormatJson,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct FormatJson {
    format_name: Option<String>,
    duration: Option<String>,
    start_time: Option<String>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct StreamJson {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u64>,
    height: Option<u64>,
    avg_frame_rate: Option<String>,
    r_frame_rate: Option<String>,
    duration: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u64>,
    disposition: DispositionJson,
    side_data_list: Vec<SideDataJson>,
    tags: TagsJson,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct DispositionJson {
    attached_pic: i64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct SideDataJson {
    /// Counter-clockwise degrees from the display matrix; integer or float depending on version.
    rotation: Option<serde_json::Number>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct TagsJson {
    /// Clockwise degrees (legacy MP4 tag).
    rotate: Option<String>,
}

/// Turns ffprobe's JSON into [`MediaInfo`] for the file at `path`.
pub(crate) fn parse_probe(json: &[u8], path: PathBuf) -> Result<MediaInfo, MediaError> {
    let probe: ProbeJson =
        serde_json::from_slice(json).map_err(|e| MediaError::InvalidProbe(e.to_string()))?;

    let video_stream = probe
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video") && s.disposition.attached_pic == 0)
        .ok_or(MediaError::NoVideoStream)?;
    let video = parse_video(video_stream)?;

    let audio = probe
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("audio"))
        .map(parse_audio);

    // The video stream's own length wins: the container length also covers longer audio, which
    // would add frames that do not exist. Matroska has no stream duration, hence the fallback.
    let duration = [&video_stream.duration, &probe.format.duration]
        .into_iter()
        .filter_map(|d| d.as_deref().and_then(Tick::parse_decimal_secs))
        .find(|d| *d > Tick::ZERO)
        .ok_or(MediaError::UnknownDuration)?;

    let start = probe
        .format
        .start_time
        .as_deref()
        .and_then(Tick::parse_decimal_secs)
        .unwrap_or(Tick::ZERO);

    Ok(MediaInfo {
        path,
        container: probe.format.format_name.unwrap_or_default(),
        start,
        duration,
        video,
        audio,
    })
}

fn parse_video(s: &StreamJson) -> Result<VideoInfo, MediaError> {
    let dimension = |v: Option<u64>, what: &str| {
        v.and_then(|v| u32::try_from(v).ok())
            .filter(|v| *v > 0)
            .ok_or_else(|| MediaError::InvalidProbe(format!("video stream has no valid {what}")))
    };
    let frame_rate = [&s.avg_frame_rate, &s.r_frame_rate]
        .into_iter()
        .find_map(|r| r.as_deref().and_then(parse_rate))
        .ok_or_else(|| MediaError::InvalidProbe("video stream has no valid frame rate".into()))?;

    Ok(VideoInfo {
        codec: s.codec_name.clone().unwrap_or_default(),
        width: dimension(s.width, "width")?,
        height: dimension(s.height, "height")?,
        frame_rate,
        rotation: rotation_cw(s),
    })
}

fn parse_audio(s: &StreamJson) -> AudioInfo {
    AudioInfo {
        codec: s.codec_name.clone().unwrap_or_default(),
        sample_rate: s
            .sample_rate
            .as_deref()
            .and_then(|r| r.trim().parse().ok())
            .filter(|r| *r > 0),
        channels: s
            .channels
            .and_then(|c| u32::try_from(c).ok())
            .filter(|c| *c > 0),
    }
}

/// `"30000/1001"` → 30000/1001; `"0/0"`, garbage and implausible rates → `None`.
fn parse_rate(text: &str) -> Option<FrameRate> {
    let (num, den) = text.trim().split_once('/')?;
    let rate = FrameRate::new(num.parse().ok()?, den.parse().ok()?)?;
    // num / den <= MAX_FPS, without division.
    (u64::from(rate.num()) <= u64::from(rate.den()) * u64::from(MAX_FPS)).then_some(rate)
}

/// Clockwise display rotation normalised to 0/90/180/270.
fn rotation_cw(s: &StreamJson) -> u16 {
    let from_side_data = s
        .side_data_list
        .iter()
        .find_map(|d| d.rotation.as_ref())
        .and_then(number_to_degrees)
        .map(|ccw| -ccw);
    let from_tag = || {
        s.tags
            .rotate
            .as_deref()
            .and_then(|r| r.trim().parse::<i64>().ok())
    };
    let degrees = from_side_data.or_else(from_tag).unwrap_or(0);
    // Snap to the nearest quarter turn; rem_euclid keeps it in 0..360.
    let quarter_turns = (degrees.rem_euclid(360) + 45) / 90 % 4;
    match quarter_turns {
        1 => 90,
        2 => 180,
        3 => 270,
        _ => 0,
    }
}

fn number_to_degrees(n: &serde_json::Number) -> Option<i64> {
    if let Some(i) = n.as_i64() {
        return Some(i.rem_euclid(360));
    }
    let f = n
        .as_f64()
        .filter(|f| f.is_finite())?
        .rem_euclid(360.0)
        .round();
    // In 0..=360 after rem_euclid + round, so the cast cannot truncate.
    #[allow(clippy::cast_possible_truncation)]
    Some(f as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TPS: i64 = microcut_time::TICKS_PER_SECOND;

    fn parse(json: &str) -> Result<MediaInfo, MediaError> {
        parse_probe(json.as_bytes(), PathBuf::from("clip.mp4"))
    }

    const PHONE_CLIP: &str = r#"{
      "streams": [
        { "index": 0, "codec_name": "h264", "codec_type": "video", "width": 1920, "height": 1080,
          "r_frame_rate": "30000/1001", "avg_frame_rate": "30000/1001", "duration": "10.010000",
          "disposition": { "default": 1, "attached_pic": 0 },
          "side_data_list": [ { "side_data_type": "Display Matrix", "rotation": -90 } ] },
        { "index": 1, "codec_name": "aac", "codec_type": "audio", "sample_rate": "48000",
          "channels": 2, "disposition": { "attached_pic": 0 } }
      ],
      "format": { "format_name": "mov,mp4,m4a,3gp,3g2,mj2", "start_time": "0.000000",
                  "duration": "10.026667", "size": "123456" }
    }"#;

    #[test]
    fn parses_typical_phone_clip() {
        let info = parse(PHONE_CLIP).unwrap();
        assert_eq!(info.container, "mov,mp4,m4a,3gp,3g2,mj2");
        // The video stream (10.01 s), not the longer container (10.026667 s, audio overhang).
        assert_eq!(
            info.duration,
            Tick::parse_decimal_secs("10.010000").unwrap()
        );
        assert_eq!(info.start, Tick::ZERO);
        assert_eq!(info.video.codec, "h264");
        assert_eq!((info.video.width, info.video.height), (1920, 1080));
        assert_eq!(
            info.video.frame_rate,
            FrameRate::new(30_000, 1_001).unwrap()
        );
        assert_eq!(info.video.rotation, 90);
        assert_eq!(info.video.display_size(), (1080, 1920));
        assert_eq!(info.frame_count(), 300);
        let audio = info.audio.unwrap();
        assert_eq!(
            (audio.codec.as_str(), audio.sample_rate, audio.channels),
            ("aac", Some(48_000), Some(2))
        );
    }

    #[test]
    fn skips_cover_art_and_uses_stream_duration_as_fallback() {
        let info = parse(
            r#"{ "streams": [
                { "codec_type": "video", "codec_name": "mjpeg", "width": 600, "height": 600,
                  "avg_frame_rate": "0/0", "r_frame_rate": "90000/1",
                  "disposition": { "attached_pic": 1 } },
                { "codec_type": "video", "codec_name": "vp9", "width": 640, "height": 360,
                  "avg_frame_rate": "0/0", "r_frame_rate": "25/1", "duration": "4.5" } ],
              "format": { "format_name": "matroska,webm", "duration": "N/A", "start_time": "1.4" } }"#,
        )
        .unwrap();
        assert_eq!(info.video.codec, "vp9");
        assert_eq!(info.video.frame_rate, FrameRate::new(25, 1).unwrap());
        assert_eq!(info.duration, Tick(9 * TPS / 2));
        assert_eq!(info.start, Tick(14 * TPS / 10));
        assert!(info.audio.is_none());
        assert_eq!(info.video.rotation, 0);
    }

    #[test]
    fn legacy_rotate_tag_and_float_rotation() {
        let tag = r#"{ "streams": [ { "codec_type": "video", "width": 4, "height": 2,
            "avg_frame_rate": "24/1", "tags": { "rotate": "270" } } ],
            "format": { "duration": "1" } }"#;
        assert_eq!(parse(tag).unwrap().video.rotation, 270);

        let float = r#"{ "streams": [ { "codec_type": "video", "width": 4, "height": 2,
            "avg_frame_rate": "24/1", "side_data_list": [ { "rotation": 179.6 } ] } ],
            "format": { "duration": "1" } }"#;
        assert_eq!(parse(float).unwrap().video.rotation, 180);
    }

    #[test]
    fn hostile_or_incomplete_json_is_an_error_not_a_crash() {
        let cases = [
            ("", "invalid"),
            ("null", "invalid"),
            ("[]", "no video"), // serde reads an empty sequence as a struct of defaults
            ("{}", "no video"),
            (r#"{"streams": [{"codec_type": "audio"}]}"#, "no video"),
            (
                r#"{"streams": [{"codec_type": "video", "width": 0, "height": 10,
                "avg_frame_rate": "25/1"}], "format": {"duration": "1"}}"#,
                "invalid",
            ),
            (
                r#"{"streams": [{"codec_type": "video", "width": 99999999999, "height": 10,
                "avg_frame_rate": "25/1"}], "format": {"duration": "1"}}"#,
                "invalid",
            ),
            (
                r#"{"streams": [{"codec_type": "video", "width": 10, "height": 10,
                "avg_frame_rate": "1/0", "r_frame_rate": "-5/1"}],
                "format": {"duration": "1"}}"#,
                "invalid",
            ),
            (
                r#"{"streams": [{"codec_type": "video", "width": 10, "height": 10,
                "avg_frame_rate": "25/1"}], "format": {"duration": "-3"}}"#,
                "duration",
            ),
            (
                r#"{"streams": [{"codec_type": "video", "width": 10, "height": 10,
                "avg_frame_rate": "25/1"}], "format": {"duration": "1e400"}}"#,
                "duration",
            ),
            (
                r#"{"streams": [{"codec_type": "video", "width": "wide"}]}"#,
                "invalid",
            ),
        ];
        for (json, expected) in cases {
            let err = parse(json).unwrap_err();
            let ok = match expected {
                "invalid" => matches!(err, MediaError::InvalidProbe(_)),
                "no video" => matches!(err, MediaError::NoVideoStream),
                "duration" => matches!(err, MediaError::UnknownDuration),
                _ => false,
            };
            assert!(ok, "{json} → {err:?}");
        }
    }

    #[test]
    fn rate_parsing_rejects_garbage() {
        assert_eq!(parse_rate("24000/1001"), FrameRate::new(24_000, 1_001));
        for bad in [
            "0/0",
            "25",
            "25/0",
            "/1",
            "a/b",
            "90000/1",
            "-25/1",
            "4294967296/1",
        ] {
            assert_eq!(parse_rate(bad), None, "{bad}");
        }
        assert_eq!(parse_rate("1000/1"), FrameRate::new(1_000, 1));
    }
}
