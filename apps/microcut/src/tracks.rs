//! Audio track selection: small numbered buttons `[0] [1] [2]` next to the volume control,
//! shown only when the video has more than one audio track.

use eframe::egui::{self, Align2, FontId, Sense, Stroke, Vec2};
use microcut_media::AudioInfo;

/// `aac 48 kHz stereo`
pub fn describe(audio: &AudioInfo) -> String {
    let mut text = audio.codec.clone();
    if let Some(rate) = audio.sample_rate {
        text.push_str(&format!(" {} kHz", f64::from(rate) / 1000.0));
    }
    match audio.channels {
        Some(1) => text.push_str(" mono"),
        Some(2) => text.push_str(" stereo"),
        Some(n) => text.push_str(&format!(" {n} ch")),
        None => {}
    }
    text
}

/// `Audio track 1: Music (eng) · aac 48 kHz stereo`
pub fn tooltip(track: usize, audio: &AudioInfo) -> String {
    let mut name = audio.title.clone().unwrap_or_default();
    if let Some(language) = &audio.language {
        if name.is_empty() {
            name = language.clone();
        } else {
            name.push_str(&format!(" ({language})"));
        }
    }
    let description = describe(audio);
    if name.is_empty() {
        format!("Audio track {track}: {description}")
    } else {
        format!("Audio track {track}: {name} · {description}")
    }
}

/// The track buttons, laid out for a right-to-left layout (so they read `[0] [1] [2]`).
/// Returns the track that was clicked.
pub fn buttons(ui: &mut egui::Ui, tracks: &[AudioInfo], selected: Option<usize>) -> Option<usize> {
    if tracks.len() < 2 {
        return None;
    }
    let mut clicked = None;
    for (track, audio) in tracks.iter().enumerate().rev() {
        let (rect, response) = ui.allocate_exact_size(Vec2::new(18.0, 16.0), Sense::click());
        let response = response.on_hover_text(tooltip(track, audio));
        if response.clicked() {
            clicked = Some(track);
        }

        let visuals = ui.visuals();
        let active = selected == Some(track);
        let (fill, text) = if active {
            (visuals.selection.bg_fill, visuals.strong_text_color())
        } else if response.hovered() {
            (visuals.widgets.hovered.weak_bg_fill, visuals.text_color())
        } else {
            (egui::Color32::TRANSPARENT, visuals.weak_text_color())
        };
        let painter = ui.painter();
        painter.rect(
            rect,
            3.0,
            fill,
            // The theme's widget stroke is transparent in dark mode; the weak text colour is not.
            Stroke::new(1.0, visuals.weak_text_color().gamma_multiply(0.6)),
            egui::StrokeKind::Inside,
        );
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            track.to_string(),
            FontId::monospace(11.0),
            text,
        );
    }
    clicked
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(title: Option<&str>, language: Option<&str>) -> AudioInfo {
        AudioInfo {
            codec: "aac".to_owned(),
            sample_rate: Some(48_000),
            channels: Some(2),
            title: title.map(str::to_owned),
            language: language.map(str::to_owned),
        }
    }

    #[test]
    fn tooltips_name_the_track() {
        assert_eq!(
            tooltip(0, &audio(Some("Mic"), None)),
            "Audio track 0: Mic · aac 48 kHz stereo"
        );
        assert_eq!(
            tooltip(1, &audio(Some("Music"), Some("eng"))),
            "Audio track 1: Music (eng) · aac 48 kHz stereo"
        );
        assert_eq!(
            tooltip(2, &audio(None, Some("deu"))),
            "Audio track 2: deu · aac 48 kHz stereo"
        );
        assert_eq!(
            tooltip(3, &audio(None, None)),
            "Audio track 3: aac 48 kHz stereo"
        );
    }

    #[test]
    fn descriptions() {
        let mut a = audio(None, None);
        assert_eq!(describe(&a), "aac 48 kHz stereo");
        a.sample_rate = Some(44_100);
        a.channels = Some(6);
        assert_eq!(describe(&a), "aac 44.1 kHz 6 ch");
        a.sample_rate = None;
        a.channels = None;
        assert_eq!(describe(&a), "aac");
    }
}
