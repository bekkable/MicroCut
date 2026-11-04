//! Audio sync control `◀ [+150 ms] ▶`: moves the sound against the picture, typed in or in
//! steps of 50 ms with the arrows. Positive values make the sound come later.

use eframe::egui::{self, Sense, Vec2};
use microcut_engine::MAX_AUDIO_DELAY_MS;

/// Milliseconds per arrow click.
pub const STEP_MS: i32 = 50;

const HINT: &str = "Audio sync: + plays the sound later, - earlier (milliseconds). \
                    Click to type a value, drag to change it, arrows: ±50 ms. \
                    Applies to playback and export.";

/// A change of the audio sync.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyncEdit {
    /// The new value in milliseconds.
    pub ms: i32,
    /// The user is done (not in the middle of a drag): time to restart playback with it.
    pub commit: bool,
}

/// Draws the control (in either layout direction, so it always reads `◀ value ▶`). Returns the
/// change, if any.
pub fn control(ui: &mut egui::Ui, ms: i32) -> Option<SyncEdit> {
    // The first widget added is the leftmost one, or the rightmost one in a right-to-left layout.
    let first = if ui.layout().prefer_right_to_left() {
        STEP_MS
    } else {
        -STEP_MS
    };
    let mut edit = step(ui, ms, first);
    let mut value = ms;
    let response = ui
        .add(
            egui::DragValue::new(&mut value)
                .range(-MAX_AUDIO_DELAY_MS..=MAX_AUDIO_DELAY_MS)
                .speed(1.0)
                .min_size(Vec2::new(64.0, 0.0))
                .custom_formatter(|v, _| format_ms(v))
                .custom_parser(parse_ms)
                .suffix(" ms"),
        )
        .on_hover_text(HINT);
    let commit = (response.changed() && !response.dragged()) || response.drag_stopped();
    if response.changed() || commit {
        edit = edit.or(Some(SyncEdit {
            ms: clamp(value),
            commit,
        }));
    }
    step(ui, ms, -first).or(edit)
}

/// An arrow that moves `ms` by `by`.
fn step(ui: &mut egui::Ui, ms: i32, by: i32) -> Option<SyncEdit> {
    arrow(ui, by).clicked().then(|| SyncEdit {
        ms: clamp(ms.saturating_add(by)),
        commit: true,
    })
}

fn clamp(ms: i32) -> i32 {
    ms.clamp(-MAX_AUDIO_DELAY_MS, MAX_AUDIO_DELAY_MS)
}

/// A small drawn triangle button (left for `by < 0`). Never takes keyboard focus, so the
/// navigation keys keep working after a click.
fn arrow(ui: &mut egui::Ui, by: i32) -> egui::Response {
    let size = Vec2::new(14.0, ui.spacing().interact_size.y);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let hint = if by < 0 {
        format!("Sound {} ms earlier", by.unsigned_abs())
    } else {
        format!("Sound {by} ms later")
    };
    let response = response.on_hover_text(hint);
    let visuals = ui.style().interact(&response);
    let c = rect.center();
    let (tip, back) = if by < 0 { (-4.0, 3.0) } else { (4.0, -3.0) };
    let points = vec![
        c + egui::vec2(back, -5.0),
        c + egui::vec2(back, 5.0),
        c + egui::vec2(tip, 0.0),
    ];
    ui.painter().add(egui::Shape::convex_polygon(
        points,
        visuals.fg_stroke.color,
        egui::Stroke::NONE,
    ));
    response
}

/// `0`, `+150`, `-40` (the suffix ` ms` is the widget's).
fn format_ms(value: f64) -> String {
    let ms = value.round() as i64;
    if ms == 0 {
        "0".to_owned()
    } else {
        format!("{ms:+}")
    }
}

/// What the user typed: `150`, `+150`, `-150 ms`, `−150` (typographic minus), `12.6` (rounded).
fn parse_ms(text: &str) -> Option<f64> {
    let text = text.trim().replace('−', "-");
    let number = text.strip_suffix("ms").unwrap_or(&text).trim();
    // `f64` takes one sign itself; a `+` must not come before another sign.
    let number = match number.strip_prefix('+') {
        Some(rest) if rest.starts_with(['+', '-']) => return None,
        Some(rest) => rest,
        None => number,
    };
    let value: f64 = number.parse().ok()?;
    value.is_finite().then(|| value.round())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_show_their_sign() {
        assert_eq!(format_ms(0.0), "0");
        assert_eq!(format_ms(150.0), "+150");
        assert_eq!(format_ms(-40.0), "-40");
        assert_eq!(format_ms(-0.4), "0");
    }

    #[test]
    fn typed_values_are_read() {
        assert_eq!(parse_ms("150"), Some(150.0));
        assert_eq!(parse_ms(" +150 "), Some(150.0));
        assert_eq!(parse_ms("-150 ms"), Some(-150.0));
        assert_eq!(parse_ms("−75ms"), Some(-75.0));
        assert_eq!(parse_ms("12.6"), Some(13.0));
        assert_eq!(parse_ms(&format_ms(-40.0)), Some(-40.0));
        assert_eq!(parse_ms(""), None);
        assert_eq!(parse_ms("abc"), None);
        assert_eq!(parse_ms("inf"), None);
        assert_eq!(parse_ms("NaN"), None);
        assert_eq!(parse_ms("++5"), None);
        assert_eq!(parse_ms("+-5"), None);
    }

    #[test]
    fn steps_stay_in_range() {
        assert_eq!(clamp(MAX_AUDIO_DELAY_MS + STEP_MS), MAX_AUDIO_DELAY_MS);
        assert_eq!(clamp(i32::MIN), -MAX_AUDIO_DELAY_MS);
    }
}
