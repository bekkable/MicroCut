//! Output volume: a small VLC-style control (speaker to mute, wedge to set the level, wheel to
//! adjust) and the gain it hands to the sound device callback.

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use eframe::egui::{self, Pos2, Rect, Sense, Stroke, Vec2};

/// Level change per wheel notch (about 50 points of scrolling).
const WHEEL_STEP: f32 = 0.05;
const WHEEL_NOTCH_POINTS: f32 = 50.0;

/// The volume setting and the gain derived from it, shared with playback.
pub struct Volume {
    /// 0.0..=1.0 as shown (100 % = the file's own level).
    level: f32,
    muted: bool,
    /// `f32` bits of the current gain, read by the sound callback.
    gain: Arc<AtomicU32>,
}

impl Default for Volume {
    fn default() -> Volume {
        let volume = Volume {
            level: 1.0,
            muted: false,
            gain: Arc::new(AtomicU32::new(0)),
        };
        volume.publish();
        volume
    }
}

impl Volume {
    /// Handle the sound callback reads the gain from (see [`gain_of`]).
    pub fn gain(&self) -> Arc<AtomicU32> {
        Arc::clone(&self.gain)
    }

    fn set_level(&mut self, level: f32) {
        if level.is_finite() {
            self.level = level.clamp(0.0, 1.0);
            self.muted = false;
            self.publish();
        }
    }

    fn publish(&self) {
        self.gain.store(
            gain_for(self.level, self.muted).to_bits(),
            Ordering::Release,
        );
    }

    /// Shows the control. Must be called in a right-to-left layout (it adds its parts from the
    /// right: percentage, wedge, speaker).
    pub fn ui(&mut self, ui: &mut egui::Ui) {
        let percent = (self.level * 100.0).round();
        ui.label(
            egui::RichText::new(format!("{percent:>3} %"))
                .small()
                .monospace()
                .weak(),
        );

        let (wedge, wedge_response) =
            ui.allocate_exact_size(Vec2::new(56.0, 14.0), Sense::click_and_drag());
        if wedge_response.is_pointer_button_down_on()
            && let Some(pos) = wedge_response.interact_pointer_pos()
        {
            self.set_level((pos.x - wedge.left()) / wedge.width());
        }

        let (speaker, speaker_response) =
            ui.allocate_exact_size(Vec2::new(18.0, 16.0), Sense::click());
        if speaker_response.clicked() {
            self.muted = !self.muted;
            self.publish();
        }

        let hovered = wedge_response.hovered() || speaker_response.hovered();
        if hovered {
            let scroll = ui.input(|i| i.smooth_scroll_delta().y);
            if scroll != 0.0 {
                self.set_level(self.level + scroll / WHEEL_NOTCH_POINTS * WHEEL_STEP);
            }
        }
        let hint = if self.muted {
            format!("Muted (volume {percent} %) — click the speaker to unmute")
        } else {
            format!("Volume {percent} % — click the speaker to mute")
        };
        wedge_response.on_hover_text(&hint);
        speaker_response.on_hover_text(&hint);

        self.paint(ui, wedge, speaker);
    }

    fn paint(&self, ui: &egui::Ui, wedge: Rect, speaker: Rect) {
        let visuals = ui.visuals();
        let painter = ui.painter();
        let level_color = if self.muted {
            visuals.weak_text_color()
        } else {
            visuals.selection.bg_fill
        };

        // Wedge: the full triangle, then the part up to the level.
        let (l, r, t, b) = (wedge.left(), wedge.right(), wedge.top(), wedge.bottom());
        let full = vec![Pos2::new(l, b), Pos2::new(r, b), Pos2::new(r, t)];
        painter.add(egui::Shape::convex_polygon(
            full,
            visuals.widgets.inactive.bg_fill,
            Stroke::NONE,
        ));
        if self.level > 0.0 {
            let x = l + wedge.width() * self.level;
            let y = b - wedge.height() * self.level;
            let part = vec![Pos2::new(l, b), Pos2::new(x, b), Pos2::new(x, y)];
            painter.add(egui::Shape::convex_polygon(part, level_color, Stroke::NONE));
        }

        // Speaker: body and cone, then waves or a cross.
        let color = visuals.text_color();
        let c = speaker.center();
        let body = Rect::from_center_size(c + Vec2::new(-5.0, 0.0), Vec2::new(3.0, 5.0));
        painter.rect_filled(body, 0.0, color);
        let cone = vec![
            Pos2::new(body.right(), body.top()),
            Pos2::new(body.right() + 4.0, c.y - 5.0),
            Pos2::new(body.right() + 4.0, c.y + 5.0),
            Pos2::new(body.right(), body.bottom()),
        ];
        painter.add(egui::Shape::convex_polygon(cone, color, Stroke::NONE));
        let stroke = Stroke::new(1.2, color);
        let x = c.x + 3.0;
        if self.muted || self.level <= 0.0 {
            painter.line_segment(
                [Pos2::new(x, c.y - 3.0), Pos2::new(x + 5.0, c.y + 3.0)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(x, c.y + 3.0), Pos2::new(x + 5.0, c.y - 3.0)],
                stroke,
            );
        } else {
            painter.vline(x + 1.0, (c.y - 2.0)..=(c.y + 2.0), stroke);
            if self.level > 0.5 {
                painter.vline(x + 4.0, (c.y - 4.5)..=(c.y + 4.5), stroke);
            }
        }
    }
}

/// Gain for a shown level: squared, so the control feels even to the ear; 0 when muted.
fn gain_for(level: f32, muted: bool) -> f32 {
    if muted || !level.is_finite() {
        0.0
    } else {
        level.clamp(0.0, 1.0).powi(2)
    }
}

/// Reads the gain published by a [`Volume`].
pub fn gain_of(handle: &AtomicU32) -> f32 {
    let gain = f32::from_bits(handle.load(Ordering::Acquire));
    if gain.is_finite() {
        gain.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gain_follows_level_and_mute() {
        assert_eq!(gain_for(1.0, false), 1.0);
        assert_eq!(gain_for(0.5, false), 0.25);
        assert_eq!(gain_for(0.8, true), 0.0);
        assert_eq!(gain_for(2.0, false), 1.0);
        assert_eq!(gain_for(f32::NAN, false), 0.0);
    }

    #[test]
    fn handle_carries_the_gain() {
        let mut volume = Volume::default();
        let handle = volume.gain();
        assert_eq!(gain_of(&handle), 1.0);
        volume.set_level(0.5);
        assert_eq!(gain_of(&handle), 0.25);
        volume.muted = true;
        volume.publish();
        assert_eq!(gain_of(&handle), 0.0);
        volume.set_level(f32::NAN);
        assert_eq!(gain_of(&handle), 0.0, "ignored, still muted");
        handle.store(f32::NAN.to_bits(), Ordering::Release);
        assert_eq!(gain_of(&handle), 0.0);
    }
}
