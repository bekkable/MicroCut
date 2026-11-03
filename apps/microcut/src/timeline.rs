//! The timeline at the bottom of the window: frame scale with the selected range on top, the
//! video strip with thumbnails in the middle, the time scale below. One cursor line runs through
//! all three.
//!
//! Mouse: wheel zooms around the pointer, Shift+wheel (or a horizontal wheel/touchpad) scrolls,
//! click/drag moves the cursor, dragging a range handle on the frame scale moves that end of the
//! range, double-click on the time scale zooms out to the whole clip.

use std::sync::Arc;

use eframe::egui::{self, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use microcut_engine::{Session, Tile, frame_marks, tiles, time_marks};
use microcut_media::MediaInfo;

use crate::preview::physical_pixels;
use crate::thumbs::Thumbnails;

const FRAME_SCALE_HEIGHT: f32 = 20.0;
const STRIP_HEIGHT: f32 = 54.0;
const TIME_SCALE_HEIGHT: f32 = 20.0;
const LABEL_FONT_SIZE: f32 = 10.5;

/// How close (points) the pointer must be to a range handle to grab it.
const HANDLE_GRAB: f64 = 7.0;
/// Labels on a scale are at least this far apart (points).
const MIN_LABEL_SPACING: f64 = 84.0;
/// Wheel distance (points) that zooms by a factor of e.
const WHEEL_ZOOM_POINTS: f64 = 200.0;
/// Thumbnails are decoded this much larger than drawn: tiles are up to 1.5 thumbnails wide.
const THUMB_OVERSIZE: f32 = 1.5;

const CURSOR_COLOR: Color32 = Color32::from_rgb(255, 86, 64);

/// What a pointer drag moves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Drag {
    Cursor,
    RangeStart,
    RangeEnd,
}

/// The three rows of the timeline.
struct Rows {
    frames: Rect,
    strip: Rect,
    times: Rect,
}

impl Rows {
    fn new(rect: Rect) -> Rows {
        let split = |top: f32, height: f32| {
            Rect::from_min_size(Pos2::new(rect.left(), top), Vec2::new(rect.width(), height))
        };
        let strip_top = rect.top() + FRAME_SCALE_HEIGHT;
        let times_top = strip_top + STRIP_HEIGHT;
        Rows {
            frames: split(rect.top(), FRAME_SCALE_HEIGHT),
            strip: split(strip_top, STRIP_HEIGHT),
            times: split(times_top, TIME_SCALE_HEIGHT),
        }
    }
}

/// Timeline widget state (what is being dragged).
#[derive(Default)]
pub struct Timeline {
    drag: Option<Drag>,
}

impl Timeline {
    /// Shows the timeline across the available width and applies mouse input to `session`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        thumbs: Option<&mut Thumbnails>,
    ) {
        let height = FRAME_SCALE_HEIGHT + STRIP_HEIGHT + TIME_SCALE_HEIGHT;
        let size = Vec2::new(ui.available_width(), height);
        let (rect, response) = ui.allocate_exact_size(size, Sense::click_and_drag());
        session.view_mut().set_width(f64::from(rect.width()));
        let rows = Rows::new(rect);

        wheel(ui, rect, session);
        self.pointer(ui, &response, &rows, session);

        let painter = ui.painter_at(rect);
        paint_frame_scale(ui, &painter, &rows, session);
        paint_strip(ui, &painter, &rows, session, thumbs);
        paint_time_scale(ui, &painter, &rows, session);
        paint_cursor(&painter, rect, &rows, session);
    }

    fn pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rows: &Rows,
        session: &mut Session,
    ) {
        let left = rows.frames.left();
        let width = f64::from(rows.frames.width());

        if response.double_clicked()
            && response
                .interact_pointer_pos()
                .is_some_and(|pos| rows.times.contains(pos))
        {
            session.view_mut().fit();
        }

        let pressed =
            response.is_pointer_button_down_on() && ui.input(|i| i.pointer.primary_down());
        let pos = response.interact_pointer_pos();
        let (true, Some(pos)) = (pressed, pos) else {
            self.drag = None;
            if let Some(hover) = response.hover_pos()
                && handle_at(rows, hover, session).is_some()
            {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            return;
        };

        let drag = *self
            .drag
            .get_or_insert_with(|| handle_at(rows, pos, session).unwrap_or(Drag::Cursor));
        if drag != Drag::Cursor {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
        }

        // Dragging past either end scrolls, faster the further out the pointer is.
        let x = f64::from(pos.x - left);
        let outside = if x < 0.0 {
            x
        } else if x > width {
            x - width
        } else {
            0.0
        };
        if outside != 0.0 {
            session.view_mut().pan(outside * 0.25);
            ui.ctx().request_repaint();
        }
        let x = x.clamp(0.0, width);

        let view = *session.view();
        match drag {
            Drag::Cursor => session.set_cursor(view.frame_at(x)),
            Drag::RangeStart => {
                let start = session.set_range_start(view.boundary_at(x));
                session.set_cursor(start);
            }
            Drag::RangeEnd => {
                let end = session.set_range_end(view.boundary_at(x).saturating_sub(1));
                session.set_cursor(end);
            }
        }
    }
}

/// Wheel: zoom around the pointer; horizontal wheel or Shift+wheel: scroll.
fn wheel(ui: &egui::Ui, rect: Rect, session: &mut Session) {
    if !ui.rect_contains_pointer(rect) {
        return;
    }
    let Some(pointer) = ui.input(|i| i.pointer.hover_pos()) else {
        return;
    };
    let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta(), i.zoom_delta()));
    let x = f64::from(pointer.x - rect.left());
    let view = session.view_mut();
    if scroll.y != 0.0 {
        // Wheel up (content moves down) zooms in.
        view.zoom_at(x, (-f64::from(scroll.y) / WHEEL_ZOOM_POINTS).exp());
    }
    if zoom != 1.0 && zoom > 0.0 {
        view.zoom_at(x, 1.0 / f64::from(zoom));
    }
    if scroll.x != 0.0 {
        view.pan(-f64::from(scroll.x));
    }
}

/// The range handle under `pos` on the frame scale, if any.
fn handle_at(rows: &Rows, pos: Pos2, session: &Session) -> Option<Drag> {
    if !rows.frames.contains(pos) {
        return None;
    }
    let view = session.view();
    let range = session.range();
    let x = f64::from(pos.x - rows.frames.left());
    let start = view.x_of(range.start() as f64);
    let end = view.x_of(range.end().saturating_add(1) as f64);
    let (to_start, to_end) = ((x - start).abs(), (x - end).abs());
    if to_start.min(to_end) > HANDLE_GRAB {
        None
    } else if to_start < to_end || (to_start == to_end && x <= start) {
        Some(Drag::RangeStart)
    } else {
        Some(Drag::RangeEnd)
    }
}

/// Screen x for a view x, kept within a sane distance of the timeline (huge coordinates upset
/// tessellation; everything outside is clipped anyway).
fn screen_x(rect: Rect, x: f64) -> f32 {
    let width = f64::from(rect.width());
    rect.left() + x.clamp(-1_000.0, width + 1_000.0) as f32
}

fn paint_frame_scale(ui: &egui::Ui, painter: &egui::Painter, rows: &Rows, session: &Session) {
    let visuals = ui.visuals();
    let row = rows.frames;
    painter.rect_filled(row, 0.0, visuals.faint_bg_color);

    // Selected range with its two handles.
    let view = session.view();
    let range = session.range();
    let x0 = screen_x(row, view.x_of(range.start() as f64));
    let x1 = screen_x(row, view.x_of(range.end().saturating_add(1) as f64));
    let accent = visuals.selection.bg_fill;
    let band = Rect::from_x_y_ranges(x0..=x1, row.y_range());
    painter.rect_filled(band, 0.0, accent.gamma_multiply(0.45));
    for (x, inward) in [(x0, 1.0), (x1, -1.0)] {
        let handle = Rect::from_x_y_ranges(
            x.min(x + 5.0 * inward)..=x.max(x + 5.0 * inward),
            row.y_range(),
        );
        painter.rect_filled(handle, 0.0, accent);
        painter.vline(x, row.top()..=rows.strip.bottom(), Stroke::new(1.5, accent));
    }

    let text = visuals.text_color();
    let weak = visuals.weak_text_color();
    for mark in frame_marks(view, MIN_LABEL_SPACING) {
        let x = screen_x(row, mark.x);
        let tick = if mark.label.is_some() { 7.0 } else { 3.0 };
        painter.vline(
            x,
            (row.bottom() - tick)..=row.bottom(),
            Stroke::new(1.0, weak),
        );
        if let Some(label) = mark.label {
            let at = Pos2::new(x + 3.0, row.top() + 2.0);
            scale_label(painter, row, (at, egui::Align2::LEFT_TOP), label, text);
        }
    }
}

fn paint_time_scale(ui: &egui::Ui, painter: &egui::Painter, rows: &Rows, session: &Session) {
    let visuals = ui.visuals();
    let row = rows.times;
    painter.rect_filled(row, 0.0, visuals.faint_bg_color);
    let text = visuals.text_color();
    let weak = visuals.weak_text_color();
    for mark in time_marks(session.view(), session.frame_rate(), MIN_LABEL_SPACING) {
        let x = screen_x(row, mark.x);
        let tick = if mark.label.is_some() { 7.0 } else { 3.0 };
        painter.vline(x, row.top()..=(row.top() + tick), Stroke::new(1.0, weak));
        if let Some(label) = mark.label {
            let at = Pos2::new(x + 3.0, row.bottom() - 2.0);
            scale_label(painter, row, (at, egui::Align2::LEFT_BOTTOM), label, text);
        }
    }
}

/// Draws a scale label anchored at `pos`, unless it would be cut off at either end of the row.
fn scale_label(
    painter: &egui::Painter,
    row: Rect,
    (pos, anchor): (Pos2, egui::Align2),
    label: String,
    color: Color32,
) {
    let galley = painter.layout_no_wrap(label, FontId::monospace(LABEL_FONT_SIZE), color);
    let rect = anchor.anchor_size(pos, galley.size());
    if rect.left() >= row.left() && rect.right() <= row.right() {
        painter.galley(rect.min, galley, color);
    }
}

fn paint_strip(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rows: &Rows,
    session: &Session,
    thumbs: Option<&mut Thumbnails>,
) {
    let row = rows.strip;
    painter.rect_filled(row, 0.0, Color32::BLACK);
    let media = session.media();
    let thumb_width = thumb_width(media);
    let layout = tiles(session.view(), f64::from(thumb_width));

    if let Some(thumbs) = thumbs {
        request(ui, thumbs, media, &layout, thumb_width);
        for tile in &layout {
            let tile_rect = Rect::from_x_y_ranges(
                screen_x(row, tile.x0)..=screen_x(row, tile.x1),
                row.y_range(),
            );
            match thumbs.get(tile.frame) {
                Some(texture) => paint_thumb(painter, texture, tile_rect, thumb_width),
                None => {
                    painter.rect_filled(tile_rect.shrink(1.0), 0.0, Color32::from_gray(34));
                }
            }
        }
    }

    // Dim what lies outside the selected range.
    let view = session.view();
    let range = session.range();
    let x0 = screen_x(row, view.x_of(range.start() as f64));
    let x1 = screen_x(row, view.x_of(range.end().saturating_add(1) as f64));
    let shade = Color32::from_black_alpha(150);
    if x0 > row.left() {
        painter.rect_filled(
            Rect::from_x_y_ranges(row.left()..=x0, row.y_range()),
            0.0,
            shade,
        );
    }
    if x1 < row.right() {
        painter.rect_filled(
            Rect::from_x_y_ranges(x1..=row.right(), row.y_range()),
            0.0,
            shade,
        );
    }
}

/// Takes finished thumbnails and queues those of the visible tiles.
fn request(
    ui: &egui::Ui,
    thumbs: &mut Thumbnails,
    media: &Arc<MediaInfo>,
    layout: &[Tile],
    thumb_width: f32,
) {
    let ppp = ui.ctx().pixels_per_point();
    let size = (
        physical_pixels(thumb_width * THUMB_OVERSIZE, ppp),
        physical_pixels(STRIP_HEIGHT * THUMB_OVERSIZE, ppp),
    );
    let frames: Vec<i64> = layout.iter().map(|tile| tile.frame).collect();
    thumbs.want(ui.ctx(), media, &frames, size);
}

/// Width of a thumbnail at strip height, from the video's display aspect ratio.
fn thumb_width(media: &MediaInfo) -> f32 {
    let (w, h) = media.video.display_size();
    if w == 0 || h == 0 {
        return STRIP_HEIGHT * 16.0 / 9.0;
    }
    (STRIP_HEIGHT * w as f32 / h as f32).clamp(16.0, 1_000.0)
}

/// Draws a thumbnail into its tile: filling it (cropped) when the tile is up to 1.5 thumbnails
/// wide, otherwise at its own size in the middle of the tile (deep zoom, one frame per tile).
fn paint_thumb(
    painter: &egui::Painter,
    texture: &egui::TextureHandle,
    tile: Rect,
    thumb_width: f32,
) {
    if tile.width() <= thumb_width * THUMB_OVERSIZE + 0.5 {
        let uv = cover_uv(tile.width() / tile.height(), thumb_width / STRIP_HEIGHT);
        painter.image(texture.id(), tile, uv, Color32::WHITE);
    } else {
        let picture = Rect::from_center_size(tile.center(), Vec2::new(thumb_width, tile.height()));
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        painter.image(texture.id(), picture, full, Color32::WHITE);
        painter.vline(
            tile.left(),
            tile.y_range(),
            Stroke::new(1.0, Color32::from_gray(60)),
        );
    }
}

/// Texture coordinates that fill a tile of aspect `tile_aspect` with a picture of aspect
/// `picture_aspect` without distortion: wider tiles crop top and bottom equally, narrower tiles
/// keep the left part (where the tile's frame starts).
fn cover_uv(tile_aspect: f32, picture_aspect: f32) -> Rect {
    let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
    if !(tile_aspect > 0.0 && picture_aspect > 0.0) || !tile_aspect.is_finite() {
        return full;
    }
    if tile_aspect >= picture_aspect {
        let visible = picture_aspect / tile_aspect;
        Rect::from_min_max(
            Pos2::new(0.0, 0.5 - visible / 2.0),
            Pos2::new(1.0, 0.5 + visible / 2.0),
        )
    } else {
        Rect::from_min_max(Pos2::ZERO, Pos2::new(tile_aspect / picture_aspect, 1.0))
    }
}

fn paint_cursor(painter: &egui::Painter, rect: Rect, rows: &Rows, session: &Session) {
    let view = session.view();
    let cursor = session.cursor() as f64;
    let x0 = view.x_of(cursor);
    let x1 = view.x_of(cursor + 1.0);
    if x1 < 0.0 || x0 > view.width() {
        return;
    }
    let left = screen_x(rect, x0);
    let right = screen_x(rect, x1);
    // Deep zoom: mark the whole frame on the strip.
    if right - left >= 3.0 {
        let cell = Rect::from_x_y_ranges(left..=right, rows.strip.y_range());
        painter.rect_stroke(
            cell,
            0.0,
            Stroke::new(1.5, CURSOR_COLOR),
            egui::StrokeKind::Inside,
        );
    }
    painter.vline(left, rect.y_range(), Stroke::new(1.5, CURSOR_COLOR));
    let head = vec![
        Pos2::new(left - 5.0, rect.top()),
        Pos2::new(left + 5.0, rect.top()),
        Pos2::new(left, rect.top() + 7.0),
    ];
    painter.add(egui::Shape::convex_polygon(
        head,
        CURSOR_COLOR,
        Stroke::NONE,
    ));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Rect, b: Rect) -> bool {
        (a.min - b.min).length() < 1e-5 && (a.max - b.max).length() < 1e-5
    }

    #[test]
    fn wide_tiles_crop_top_and_bottom() {
        // A tile 1.5 pictures wide shows the middle two thirds of the picture's height.
        let uv = cover_uv(1.5 * 16.0 / 9.0, 16.0 / 9.0);
        let expected = Rect::from_min_max(Pos2::new(0.0, 1.0 / 6.0), Pos2::new(1.0, 5.0 / 6.0));
        assert!(close(uv, expected), "{uv:?}");
    }

    #[test]
    fn narrow_tiles_keep_the_left_part() {
        let uv = cover_uv(0.5 * 16.0 / 9.0, 16.0 / 9.0);
        let expected = Rect::from_min_max(Pos2::ZERO, Pos2::new(0.5, 1.0));
        assert!(close(uv, expected), "{uv:?}");
    }

    #[test]
    fn odd_aspects_use_the_whole_picture() {
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        assert_eq!(cover_uv(f32::NAN, 1.0), full);
        assert_eq!(cover_uv(1.0, 0.0), full);
        assert_eq!(cover_uv(f32::INFINITY, 1.0), full);
    }
}
