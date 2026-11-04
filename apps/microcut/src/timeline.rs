//! The timeline at the bottom of the window: frame scale with the selected range on top, the
//! video strip with thumbnails in the middle, the time scale below it and a scrollbar showing
//! which part of the clip is visible. One cursor line runs through the scales and the strip.
//!
//! Mouse: wheel zooms around the pointer, Shift+wheel (or a horizontal wheel/touchpad) scrolls,
//! dragging the strip or a scale moves the view, a click sets the cursor, dragging near the
//! cursor line moves the cursor (Shift: to the frame at the nearest mark), dragging a range
//! handle on the frame scale moves that end of the range (Shift: snapped to the time scale's
//! marks), an arrow at the edge of the frame scale (shown when a range end is scrolled out of
//! view) brings that end to the edge of the view on a click and takes hold of it on a drag, the
//! scrollbar thumb can be dragged, double-click on the time scale or the scrollbar zooms out to
//! the whole clip.

use std::sync::Arc;

use eframe::egui::{self, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, Vec2};
use microcut_engine::{
    RangeEnd, Session, Side, Tile, frame_marks, snap_boundary, tiles, time_marks,
};
use microcut_media::MediaInfo;

use crate::preview::physical_pixels;
use crate::thumbs::Thumbnails;

const FRAME_SCALE_HEIGHT: f32 = 20.0;
const STRIP_HEIGHT: f32 = 54.0;
const TIME_SCALE_HEIGHT: f32 = 20.0;
const SCROLLBAR_HEIGHT: f32 = 12.0;
const LABEL_FONT_SIZE: f32 = 10.5;

/// How close (points) the pointer must be to a range handle to grab it.
const HANDLE_GRAB: f64 = 7.0;
/// How close (points) the pointer must be to the cursor line to grab it.
const CURSOR_GRAB: f64 = 6.0;
/// Width of the arrows for range ends out of view.
const ARROW_WIDTH: f32 = 18.0;
/// Narrowest scrollbar thumb (points), so it stays grabbable at deep zoom.
const MIN_THUMB: f32 = 16.0;
/// Labels on a scale are at least this far apart (points).
const MIN_LABEL_SPACING: f64 = 84.0;
/// Wheel distance (points) that zooms by a factor of e.
const WHEEL_ZOOM_POINTS: f64 = 200.0;
/// Thumbnails are decoded this much larger than drawn: tiles are up to 1.5 thumbnails wide.
const THUMB_OVERSIZE: f32 = 1.5;

const CURSOR_COLOR: Color32 = Color32::from_rgb(255, 86, 64);

/// What a pointer drag moves.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Drag {
    /// The view, with the content under the pointer.
    Pan,
    RangeStart,
    RangeEnd,
    /// The cursor (grabbed near its line).
    Cursor,
    /// The scrollbar thumb; `grab` is the pointer's distance (in frames) from the left edge of
    /// the view.
    Scrollbar {
        grab: f64,
    },
}

/// The rows of the timeline.
struct Rows {
    frames: Rect,
    strip: Rect,
    times: Rect,
    scrollbar: Rect,
}

impl Rows {
    fn new(rect: Rect) -> Rows {
        let split = |top: f32, height: f32| {
            Rect::from_min_size(Pos2::new(rect.left(), top), Vec2::new(rect.width(), height))
        };
        let strip_top = rect.top() + FRAME_SCALE_HEIGHT;
        let times_top = strip_top + STRIP_HEIGHT;
        let scrollbar_top = times_top + TIME_SCALE_HEIGHT;
        Rows {
            frames: split(rect.top(), FRAME_SCALE_HEIGHT),
            strip: split(strip_top, STRIP_HEIGHT),
            times: split(times_top, TIME_SCALE_HEIGHT),
            scrollbar: split(scrollbar_top, SCROLLBAR_HEIGHT),
        }
    }

    /// The scales and the strip (where the cursor line runs).
    fn content(&self) -> Rect {
        self.frames.union(self.times)
    }
}

/// Timeline widget state (what is being dragged).
#[derive(Default)]
pub struct Timeline {
    drag: Option<Drag>,
}

impl Timeline {
    /// Whether the pointer is dragging the view, a range handle or the scrollbar.
    pub fn is_dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// Shows the timeline across the available width and applies mouse input to `session`.
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        session: &mut Session,
        thumbs: Option<&mut Thumbnails>,
    ) {
        let height = FRAME_SCALE_HEIGHT + STRIP_HEIGHT + TIME_SCALE_HEIGHT + SCROLLBAR_HEIGHT;
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
        paint_scrollbar(ui, &painter, &rows, session, self.drag);
        paint_cursor(&painter, rows.content(), &rows, session);
    }

    fn pointer(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        rows: &Rows,
        session: &mut Session,
    ) {
        if self.drag.is_none()
            && let Some(hover) = response.hover_pos()
        {
            if let Some((side, end)) = arrow_at(rows, hover, session) {
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
                let text = match (end, side) {
                    (RangeEnd::Start, Side::Left) => {
                        "Click: range start to the left edge. Drag: move the range start."
                    }
                    (RangeEnd::Start, Side::Right) => {
                        "Click: range start to the right edge. Drag: move the range start."
                    }
                    (RangeEnd::End, Side::Left) => {
                        "Click: range end to the left edge. Drag: move the range end."
                    }
                    (RangeEnd::End, Side::Right) => {
                        "Click: range end to the right edge. Drag: move the range end."
                    }
                };
                response.clone().on_hover_text(text);
            } else if handle_at(rows, hover, session).is_some() || cursor_at(rows, hover, session) {
                ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
        }

        if response.drag_started() {
            let origin = ui
                .input(|i| i.pointer.press_origin())
                .or(response.interact_pointer_pos());
            self.drag = origin.map(|origin| drag_kind(rows, origin, session));
        }
        if response.dragged()
            && let (Some(drag), Some(pos)) = (self.drag, response.interact_pointer_pos())
        {
            apply_drag(ui, drag, pos, response.drag_delta(), rows, session);
        } else {
            self.drag = None;
        }

        if response.clicked()
            && let Some(pos) = response.interact_pointer_pos()
        {
            click(rows, pos, session);
        }
        if response.double_clicked()
            && response
                .interact_pointer_pos()
                .is_some_and(|pos| rows.times.contains(pos) || rows.scrollbar.contains(pos))
        {
            session.view_mut().fit();
        }
    }
}

/// What a drag starting at `origin` moves.
fn drag_kind(rows: &Rows, origin: Pos2, session: &Session) -> Drag {
    if rows.scrollbar.contains(origin) {
        let view = session.view();
        let pos = scrollbar_pos(rows, origin.x, view.end());
        let on_thumb = (view.left()..=view.left() + view.span()).contains(&pos);
        // Grabbing the track outside the thumb centres the view on the pointer first.
        let grab = if on_thumb {
            pos - view.left()
        } else {
            view.span() / 2.0
        };
        return Drag::Scrollbar { grab };
    }
    // Dragging an arrow takes hold of the range end it stands for; range handles come before
    // the cursor (which often stands on one).
    match arrow_at(rows, origin, session) {
        Some((_, RangeEnd::Start)) => Drag::RangeStart,
        Some((_, RangeEnd::End)) => Drag::RangeEnd,
        None => handle_at(rows, origin, session).unwrap_or_else(|| {
            if cursor_at(rows, origin, session) {
                Drag::Cursor
            } else {
                Drag::Pan
            }
        }),
    }
}

/// The pointer's x in the view while dragging something along the timeline: past either end
/// the view scrolls, faster the further out the pointer is; the result stays inside the view.
fn drag_x(ui: &egui::Ui, pos: Pos2, rows: &Rows, session: &mut Session) -> f64 {
    let width = f64::from(rows.frames.width());
    let x = f64::from(pos.x - rows.frames.left());
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
    x.clamp(0.0, width)
}

/// The frame boundary at view x `x`; with Shift snapped to the marks of the time scale
/// (seconds etc., depending on zoom; absolute, counted from the start of the clip).
fn boundary_at(ui: &egui::Ui, x: f64, session: &Session) -> i64 {
    let view = session.view();
    if ui.input(|i| i.modifiers.shift) {
        snap_boundary(
            view,
            session.frame_rate(),
            MIN_LABEL_SPACING,
            view.pos_at(x),
        )
    } else {
        view.boundary_at(x)
    }
}

fn apply_drag(
    ui: &egui::Ui,
    drag: Drag,
    pos: Pos2,
    delta: Vec2,
    rows: &Rows,
    session: &mut Session,
) {
    match drag {
        Drag::Pan => {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            session.view_mut().pan(-f64::from(delta.x));
        }
        Drag::Scrollbar { grab } => {
            let pos = scrollbar_pos(rows, pos.x, session.view().end());
            session.view_mut().set_left(pos - grab);
        }
        Drag::Cursor => {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            let x = drag_x(ui, pos, rows, session);
            // The frame under the pointer; with Shift the frame starting at the nearest mark.
            let frame = if ui.input(|i| i.modifiers.shift) {
                boundary_at(ui, x, session)
            } else {
                session.view().frame_at(x)
            };
            session.set_cursor(frame);
        }
        Drag::RangeStart | Drag::RangeEnd => {
            ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
            let x = drag_x(ui, pos, rows, session);
            let boundary = boundary_at(ui, x, session);
            // The preview follows the handle.
            if drag == Drag::RangeStart {
                let start = session.set_range_start(boundary);
                session.set_cursor(start);
            } else {
                let end = session.set_range_end(boundary.saturating_sub(1));
                session.set_cursor(end);
            }
        }
    }
}

/// A click (press and release without moving): arrow → that range end to the edge of the view,
/// scrollbar → centre the view there, range handle → cursor to that end's frame, elsewhere →
/// cursor to the frame under the pointer.
fn click(rows: &Rows, pos: Pos2, session: &mut Session) {
    if let Some((side, _)) = arrow_at(rows, pos, session) {
        session.bring_hidden_end(side);
    } else if rows.scrollbar.contains(pos) {
        let view = session.view_mut();
        let centre = scrollbar_pos(rows, pos.x, view.end());
        let span = view.span();
        view.set_left(centre - span / 2.0);
    } else if let Some(handle) = handle_at(rows, pos, session) {
        let range = session.range();
        match handle {
            Drag::RangeStart => session.set_cursor(range.start()),
            Drag::RangeEnd => session.set_cursor(range.end()),
            Drag::Pan | Drag::Cursor | Drag::Scrollbar { .. } => {}
        }
    } else {
        let x = f64::from(pos.x - rows.frames.left());
        let frame = session.view().frame_at(x);
        session.set_cursor(frame);
    }
}

/// Frame position (in the whole clip) for a screen x on the scrollbar.
fn scrollbar_pos(rows: &Rows, x: f32, frames: f64) -> f64 {
    let width = f64::from(rows.scrollbar.width()).max(1.0);
    f64::from(x - rows.scrollbar.left()) / width * frames
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

/// The arrow (for a range end out of view) under `pos`, if any: its side and the range end it
/// stands for.
fn arrow_at(rows: &Rows, pos: Pos2, session: &Session) -> Option<(Side, RangeEnd)> {
    if !rows.frames.contains(pos) {
        return None;
    }
    let (left, right) = session.hidden_range_ends();
    if pos.x <= rows.frames.left() + ARROW_WIDTH {
        left.map(|end| (Side::Left, end))
    } else if pos.x >= rows.frames.right() - ARROW_WIDTH {
        right.map(|end| (Side::Right, end))
    } else {
        None
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

/// Whether `pos` is on the cursor: on the scales or the strip, near the cursor line or (deep
/// zoom) inside the frame it marks.
fn cursor_at(rows: &Rows, pos: Pos2, session: &Session) -> bool {
    if !rows.content().contains(pos) {
        return false;
    }
    let view = session.view();
    let cursor = session.cursor() as f64;
    let x = f64::from(pos.x - rows.frames.left());
    near_cursor(x, view.x_of(cursor), view.x_of(cursor + 1.0))
}

/// Whether view x `x` grabs a cursor whose frame spans `left..right` (view x).
fn near_cursor(x: f64, left: f64, right: f64) -> bool {
    (x - left).abs() <= CURSOR_GRAB || (left..=right).contains(&x)
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

    // Arrows for range ends scrolled out of view (click: bring that end to the cursor).
    let (left, right) = session.hidden_range_ends();
    let sides = [
        (left.is_some(), row.left(), -1.0),
        (right.is_some(), row.right() - ARROW_WIDTH, 1.0),
    ];
    for (shown, x0, direction) in sides {
        if !shown {
            continue;
        }
        let area = Rect::from_x_y_ranges(x0..=x0 + ARROW_WIDTH, row.y_range());
        painter.rect_filled(area, 0.0, accent);
        let c = area.center();
        let tip = c + Vec2::new(5.0 * direction, 0.0);
        let back = c - Vec2::new(4.0 * direction, 0.0);
        let points = vec![tip, back + Vec2::new(0.0, -6.0), back + Vec2::new(0.0, 6.0)];
        painter.add(egui::Shape::convex_polygon(
            points,
            visuals.strong_text_color(),
            Stroke::NONE,
        ));
    }
}

/// The scrollbar: the whole clip as the track, the visible part as the thumb, plus the range
/// and the cursor for orientation.
fn paint_scrollbar(
    ui: &egui::Ui,
    painter: &egui::Painter,
    rows: &Rows,
    session: &Session,
    drag: Option<Drag>,
) {
    let visuals = ui.visuals();
    let track = rows.scrollbar.shrink2(Vec2::new(0.0, 2.0));
    painter.rect_filled(track, 3.0, visuals.extreme_bg_color);

    let view = session.view();
    let frames = view.end().max(1.0);
    let at = |pos: f64| track.left() + (pos / frames * f64::from(track.width())) as f32;

    let range = session.range();
    let band = Rect::from_x_y_ranges(
        at(range.start() as f64)..=at(range.end().saturating_add(1) as f64),
        track.y_range(),
    );
    painter.rect_filled(band, 0.0, visuals.selection.bg_fill.gamma_multiply(0.35));

    let (mut x0, mut x1) = (at(view.left()), at(view.left() + view.span()));
    if x1 - x0 < MIN_THUMB {
        let c = (x0 + x1) / 2.0;
        x0 = (c - MIN_THUMB / 2.0).max(track.left());
        x1 = (x0 + MIN_THUMB).min(track.right());
    }
    let thumb = Rect::from_x_y_ranges(x0..=x1, track.y_range());
    let active = matches!(drag, Some(Drag::Scrollbar { .. }))
        || ui
            .input(|i| i.pointer.hover_pos())
            .is_some_and(|pos| thumb.contains(pos));
    let fill = if active {
        visuals.widgets.hovered.bg_fill
    } else {
        visuals.widgets.inactive.bg_fill
    };
    painter.rect(
        thumb,
        3.0,
        fill,
        visuals.widgets.inactive.bg_stroke,
        egui::StrokeKind::Inside,
    );

    let cursor = at(session.cursor() as f64);
    painter.vline(cursor, track.y_range(), Stroke::new(1.5, CURSOR_COLOR));
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
    fn the_cursor_is_grabbed_near_its_line_or_inside_its_frame() {
        // Zoomed out: the frame is a fraction of a point wide; ±6 points around the line.
        assert!(near_cursor(100.0, 100.0, 100.2));
        assert!(near_cursor(94.0, 100.0, 100.2));
        assert!(near_cursor(106.0, 100.0, 100.2));
        assert!(!near_cursor(106.5, 100.0, 100.2));
        assert!(!near_cursor(93.0, 100.0, 100.2));
        // Deep zoom: anywhere on the marked frame too.
        assert!(near_cursor(150.0, 100.0, 180.0));
        assert!(!near_cursor(181.0, 100.0, 180.0));
    }

    #[test]
    fn odd_aspects_use_the_whole_picture() {
        let full = Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0));
        assert_eq!(cover_uv(f32::NAN, 1.0), full);
        assert_eq!(cover_uv(1.0, 0.0), full);
        assert_eq!(cover_uv(f32::INFINITY, 1.0), full);
    }
}
