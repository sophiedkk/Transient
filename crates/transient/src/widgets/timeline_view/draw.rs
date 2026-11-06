//! Drawing the timeline: ruler, lane separators, clips with waveforms, playhead.
//!
//! GTK widgets draw by adding *render nodes* to a `gtk::Snapshot`: rectangles,
//! filled paths, text. GTK then renders them on the GPU. Colours come from the
//! theme (text colour, accent colour), so light, dark and high-contrast modes
//! all work without extra code.

use adw::prelude::*;
use adw::subclass::prelude::*;
use gettextrs::gettext;
use gtk::{gdk, graphene, gsk, pango};
use transient_core::clip::visible_parts;
use transient_core::convert::{f32_from_f64, f64_from_usize, i32_from_f64, u32_from_f64};
use transient_core::mix::fade_shape;
use transient_core::project::{Clip, ClipId, TrackColor};
use transient_core::ruler;
use transient_core::time::{Frames, frames_to_f64};
use transient_core::waveform::Peak;

use super::clip_waveform;
use super::{
    CLIP_INSET, CLIP_LABEL_HEIGHT, LANE_HEIGHT, LaneClip, LaneRange, LiveTake, RULER_HEIGHT,
    TimelineView,
};

/// Labelled ruler ticks are at least this far apart, in pixels.
const TICK_SPACING: f64 = 90.0;
/// Rounded corners of clips.
const CLIP_RADIUS: f32 = 6.0;
/// Clips of tracks that are not heard (muted, or another track is soloed).
const SILENT_LANE_OPACITY: f64 = 0.4;
/// The waveform uses this much of its half-height, leaving a little room.
const HEADROOM: f32 = 0.92;

/// The colours used, taken from the current theme.
#[derive(Clone, Copy)]
struct Palette {
    foreground: gdk::RGBA,
    accent: gdk::RGBA,
    /// Dark mode: named colours are drawn in their lighter shade.
    dark: bool,
}

impl Palette {
    /// The palette for a track's clips: its colour as the accent (see
    /// `Track::color`), or the system's accent colour.
    fn for_track(self, color: Option<TrackColor>) -> Self {
        let Some(color) = color else {
            return self;
        };
        Self {
            accent: accent_color(color).to_standalone_rgba(self.dark),
            ..self
        }
    }
}

/// Libadwaita's colour for a track colour: the same named colours as the
/// accent colour picker in Settings, in the theme's shades.
fn accent_color(color: TrackColor) -> adw::AccentColor {
    match color {
        TrackColor::Blue => adw::AccentColor::Blue,
        TrackColor::Teal => adw::AccentColor::Teal,
        TrackColor::Green => adw::AccentColor::Green,
        TrackColor::Yellow => adw::AccentColor::Yellow,
        TrackColor::Orange => adw::AccentColor::Orange,
        TrackColor::Red => adw::AccentColor::Red,
        TrackColor::Pink => adw::AccentColor::Pink,
        TrackColor::Purple => adw::AccentColor::Purple,
        TrackColor::Slate => adw::AccentColor::Slate,
    }
}

/// One clip to draw (see `draw_clip`).
#[derive(Clone, Copy)]
struct ClipToDraw<'a> {
    /// Its waveform and name.
    drawn: &'a LaneClip,
    /// Where it is: the whole clip, a drag preview, or a visible part.
    clip: &'a Clip,
    /// The top of its lane, in pixels.
    lane_top: f32,
    /// Whether to draw it highlighted.
    selected: bool,
}

/// What every drawing step needs to know about the view.
struct View {
    width: f32,
    start: f64,
    frames_per_pixel: f64,
}

impl View {
    /// The x position of a timeline frame.
    fn x(&self, frame: f64) -> f64 {
        (frame - self.start) / self.frames_per_pixel
    }
}

impl TimelineView {
    /// Draws everything. Called by GTK (via `snapshot` in imp.rs) whenever the
    /// widget needs repainting.
    pub(super) fn draw(&self, snapshot: &gtk::Snapshot) {
        let (width, height) = (pixels(self.width()), pixels(self.height()));
        if width <= 0.0 || height <= 0.0 {
            return;
        }
        let style = adw::StyleManager::default();
        let palette = Palette {
            foreground: self.color(),
            accent: style.accent_color().to_standalone_rgba(style.is_dark()),
            dark: style.is_dark(),
        };
        let imp = self.imp();
        let view = View {
            width,
            start: self.start_frame(),
            frames_per_pixel: imp.frames_per_pixel.get(),
        };

        self.draw_ruler(snapshot, &view, &palette);

        // Clips being dragged (one, or a selected group) are drawn at their preview
        // positions, on top of the clips in the lanes they are over. Only the
        // visible parts of clips are drawn (where clips overlap you see, and hear,
        // the top one), worked out again with the preview so clips underneath
        // reappear while dragging.
        let drag = imp.drag.borrow();
        let preview = drag.as_ref().and_then(|drag| drag.preview);
        let moved: Vec<(usize, Clip)> = drag.as_ref().map_or_else(Vec::new, |drag| {
            drag.preview
                .into_iter()
                .chain(drag.group_preview.iter().copied())
                .collect()
        });
        let is_moved = |id: ClipId| moved.iter().any(|(_, clip)| clip.id == id);
        let selection = imp.selection.borrow();
        // Fade handles belong to a single selected clip; with several, there are none.
        let handles_for = imp.selected.get().filter(|_| selection.len() <= 1);
        let lanes = imp.lanes.borrow();
        // A clip's waveform and name, wherever it is (also while dragged elsewhere).
        let drawn_clip = |id: ClipId| {
            lanes
                .iter()
                .flat_map(|lane| &lane.clips)
                .find(|clip| clip.clip.id == id)
        };
        // The selected clip (as a whole) and its lane's top, for its fade handles.
        let mut selected_clip = None;
        for (index, lane) in lanes.iter().enumerate() {
            let top = lane_top(index);
            snapshot.append_color(
                &palette.foreground.with_alpha(0.08),
                &graphene::Rect::new(0.0, top + pixels(LANE_HEIGHT) - 1.0, width, 1.0),
            );
            if !lane.audible {
                snapshot.push_opacity(SILENT_LANE_OPACITY);
            }
            // This track's clips are drawn in its colour.
            let lane_palette = palette.for_track(lane.color);
            let stack = lane_stack(&lane.clips, index, &moved);
            let parts = visible_parts(&stack);
            for &part in &parts {
                let Some(drawn) = drawn_clip(part.id) else {
                    continue;
                };
                let is_selected = selection.contains(&part.id) || is_moved(part.id);
                if handles_for == Some(part.id) {
                    selected_clip = stack
                        .iter()
                        .find(|clip| clip.id == part.id)
                        .map(|whole| (*whole, top, parts.clone(), lane_palette));
                }
                let item = ClipToDraw {
                    drawn,
                    clip: &part,
                    lane_top: top,
                    selected: is_selected,
                };
                self.draw_clip(snapshot, &view, &lane_palette, item);
            }
            if !lane.audible {
                snapshot.pop();
            }
        }
        // The time selection: the one being dragged, or else the current one.
        let selection = drag
            .as_ref()
            .and_then(|drag| drag.range_preview)
            .or(imp.time_selection.get());
        if let Some(selection) = selection {
            draw_time_selection(snapshot, &view, &palette, selection);
        }

        // Dragged below the last lane: the clip is drawn where its new track will be.
        if let Some((lane, moved)) = preview
            && lane == lanes.len()
            && let Some(drawn) = drawn_clip(moved.id)
        {
            self.draw_new_track_preview(snapshot, &view, &palette, drawn, &moved, lane);
        }

        if let Some((clip, lane_top, parts, clip_palette)) = selected_clip {
            self.draw_fade_handles(snapshot, &clip_palette, &clip, lane_top, &parts);
        }

        if let Some(take) = imp.live_take.borrow().as_ref() {
            self.draw_live_take(snapshot, &view, &palette, take, lanes.len());
        }
        if self.rate().is_some() {
            draw_playhead(snapshot, &view, &palette, imp.playhead.get(), height);
        }
        // `is_focus`: the timeline is the window's focus widget, even while the
        // window itself is not active (then GTK's `has_focus` is false).
        if self.is_focus() {
            self.draw_focus(snapshot, &palette, width, height);
        }
    }

    /// A clip dragged below the last lane, over a faint band that shows the track
    /// it will get.
    fn draw_new_track_preview(
        &self,
        snapshot: &gtk::Snapshot,
        view: &View,
        palette: &Palette,
        drawn: &LaneClip,
        moved: &Clip,
        lane: usize,
    ) {
        let top = lane_top(lane);
        snapshot.append_color(
            &palette.foreground.with_alpha(0.04),
            &graphene::Rect::new(0.0, top, view.width, pixels(LANE_HEIGHT)),
        );
        let item = ClipToDraw {
            drawn,
            clip: moved,
            lane_top: top,
            selected: true,
        };
        self.draw_clip(snapshot, view, palette, item);
    }

    /// A take being recorded, in `lane` (the empty one below the last track): a
    /// clip that grows as the recording goes on, with the take's outline as its
    /// waveform, labelled "Recording…".
    fn draw_live_take(
        &self,
        snapshot: &gtk::Snapshot,
        view: &View,
        palette: &Palette,
        take: &LiveTake,
        lane: usize,
    ) {
        let left = view.x(frames_to_f64(take.start));
        let right = view.x(frames_to_f64(take.start + take.frames));
        if right < 0.0 || left > f64::from(view.width) {
            return;
        }
        let top = lane_top(lane) + CLIP_INSET;
        let height = pixels(LANE_HEIGHT) - 2.0 * CLIP_INSET;
        let (box_left, box_right) = (
            f32_from_f64(left.max(-4.0)),
            f32_from_f64(right.min(f64::from(view.width) + 4.0)),
        );
        let bounds = graphene::Rect::new(box_left, top, (box_right - box_left).max(1.0), height);
        snapshot.push_rounded_clip(&gsk::RoundedRect::from_rect(bounds, CLIP_RADIUS));
        snapshot.append_color(&palette.accent.with_alpha(0.16), &bounds);
        snapshot.append_color(
            &palette.accent.with_alpha(0.28),
            &graphene::Rect::new(box_left, top, bounds.width(), CLIP_LABEL_HEIGHT),
        );

        // The outline: per pixel column, the loudest block it covers.
        let wave_middle = top + CLIP_LABEL_HEIGHT + (height - CLIP_LABEL_HEIGHT) / 2.0;
        let half = (height - CLIP_LABEL_HEIGHT) / 2.0;
        let block =
            f64::from(u32::try_from(transient_engine::recorder::PEAK_BLOCK).unwrap_or(1024));
        let first_column = left.max(0.0).floor();
        let last_column = right.min(f64::from(view.width)).ceil();
        let mut column = first_column;
        while column < last_column {
            let from =
                (view.start + column * view.frames_per_pixel - frames_to_f64(take.start)) / block;
            let to = from + view.frames_per_pixel / block;
            let peak = take
                .peaks
                .get(
                    block_index(from)
                        ..block_index(to)
                            .max(block_index(from) + 1)
                            .min(take.peaks.len()),
                )
                .unwrap_or_default()
                .iter()
                .fold(0.0_f32, |peak, value| peak.max(*value));
            let bar = (peak.min(1.0) * half).max(0.5);
            snapshot.append_color(
                &palette.accent,
                &graphene::Rect::new(f32_from_f64(column), wave_middle - bar, 1.0, 2.0 * bar),
            );
            column += 1.0;
        }

        let layout = self.small_text(&gettext("Recording…"));
        snapshot.save();
        snapshot.translate(&graphene::Point::new(box_left.max(0.0) + 6.0, top + 1.0));
        snapshot.append_layout(&layout, &palette.foreground);
        snapshot.restore();
        snapshot.pop();
    }

    /// With keyboard focus: a faint band on the current track (the one Up/Down
    /// and Shift+Left/Right work on) and, as GTK draws focus (only when the
    /// keyboard is in use, in the active window), an outline around the timeline.
    fn draw_focus(&self, snapshot: &gtk::Snapshot, palette: &Palette, width: f32, height: f32) {
        let lanes = self.imp().lanes.borrow().len();
        if lanes > 0 {
            let lane = self.imp().current_lane.get().min(lanes - 1);
            snapshot.append_color(
                &palette.accent.with_alpha(0.08),
                &graphene::Rect::new(0.0, lane_top(lane), width, pixels(LANE_HEIGHT)),
            );
        }
        if self.has_visible_focus() {
            let bounds = graphene::Rect::new(1.0, 1.0, width - 2.0, height - 2.0);
            snapshot.append_border(
                &gsk::RoundedRect::from_rect(bounds, 0.0),
                &[2.0; 4],
                &[palette.accent.with_alpha(0.5); 4],
            );
        }
    }

    /// Fade handles of the selected clip, dragged to change the fades (see
    /// interaction.rs). They belong to the whole clip, so they are drawn once, even
    /// when other clips hide parts of it, and only where the clip is visible.
    fn draw_fade_handles(
        &self,
        snapshot: &gtk::Snapshot,
        palette: &Palette,
        clip: &Clip,
        lane_top: f32,
        parts: &[Clip],
    ) {
        let (fade_in_x, fade_out_x) = self.fade_handle_positions(clip);
        let y = lane_top + CLIP_INSET + CLIP_LABEL_HEIGHT / 2.0;
        for x in [fade_in_x, fade_out_x] {
            if self.is_visible_at(parts, clip.id, x) {
                draw_handle(snapshot, palette, f32_from_f64(x), y);
            }
        }
    }

    /// The time ruler: a baseline, tick marks and time labels.
    fn draw_ruler(&self, snapshot: &gtk::Snapshot, view: &View, palette: &Palette) {
        let ruler_height = pixels(RULER_HEIGHT);
        snapshot.append_color(
            &palette.foreground.with_alpha(0.15),
            &graphene::Rect::new(0.0, ruler_height - 1.0, view.width, 1.0),
        );
        let Some(rate) = self.rate() else {
            return;
        };
        let ticks = ruler::ticks(
            view.start / rate,
            view.frames_per_pixel / rate,
            f64::from(view.width),
            TICK_SPACING,
        );
        let dimmed = palette.foreground.with_alpha(0.55);
        for tick in ticks {
            let x = f32_from_f64(tick.x);
            snapshot.append_color(
                &dimmed,
                &graphene::Rect::new(x, ruler_height - 7.0, 1.0, 6.0),
            );
            let layout = self.small_text(&tick.label);
            snapshot.save();
            snapshot.translate(&graphene::Point::new(x + 4.0, 5.0));
            snapshot.append_layout(&layout, &dimmed);
            snapshot.restore();
        }
    }

    /// Draws one clip: a rounded box with its name and waveform.
    ///
    /// The clip can be a whole clip, a drag preview, or the visible part of one
    /// that other clips partly cover (see `visible_parts`).
    fn draw_clip(
        &self,
        snapshot: &gtk::Snapshot,
        view: &View,
        palette: &Palette,
        item: ClipToDraw<'_>,
    ) {
        let ClipToDraw {
            drawn,
            clip,
            lane_top,
            selected,
        } = item;
        let left = view.x(frames_to_f64(clip.start));
        let right = view.x(frames_to_f64(clip.end()));
        if right < 0.0 || left > f64::from(view.width) {
            return;
        }
        // Clamp to just outside the view: GTK draws huge rectangles badly.
        let (box_left, box_right) = (
            f32_from_f64(left.max(-f64::from(CLIP_RADIUS))),
            f32_from_f64(right.min(f64::from(view.width) + f64::from(CLIP_RADIUS))),
        );
        let top = lane_top + CLIP_INSET;
        let height = pixels(LANE_HEIGHT) - 2.0 * CLIP_INSET;
        let bounds = graphene::Rect::new(box_left, top, (box_right - box_left).max(1.0), height);
        let rounded = gsk::RoundedRect::from_rect(bounds, CLIP_RADIUS);

        let (fill, strip) = if selected { (0.26, 0.45) } else { (0.16, 0.28) };
        // A clip whose audio file is missing is drawn grey, without a waveform.
        let (colour, strength) = if drawn.waveform.is_some() {
            (palette.accent, 1.0)
        } else {
            (palette.foreground, 0.6)
        };
        snapshot.push_rounded_clip(&rounded);
        snapshot.append_color(&colour.with_alpha(fill * strength), &bounds);
        snapshot.append_color(
            &colour.with_alpha(strip * strength),
            &graphene::Rect::new(box_left, top, bounds.width(), CLIP_LABEL_HEIGHT),
        );

        // The waveform, in the area below the name strip.
        let wave_top = top + CLIP_LABEL_HEIGHT;
        let half_height = (height - CLIP_LABEL_HEIGHT) / 2.0;
        let first_column = left.max(0.0).floor();
        let last_column = right.min(f64::from(view.width)).ceil();
        let columns = usize::try_from(u32_from_f64(last_column - first_column)).unwrap_or(0);
        let (peaks, gain) = clip_waveform::clip_peaks(
            drawn,
            clip,
            view.start + first_column * view.frames_per_pixel,
            view.frames_per_pixel,
            columns,
        );
        if let Some(path) = waveform_path(
            &peaks,
            f32_from_f64(first_column),
            wave_top + half_height,
            half_height,
            gain,
        ) {
            snapshot.append_fill(&path, gsk::FillRule::Winding, &palette.accent);
        }

        let edges = (f32_from_f64(left), f32_from_f64(right));
        draw_clip_fades(
            snapshot,
            view,
            palette,
            clip,
            edges,
            (wave_top, top + height),
        );

        // The name, starting at the visible left edge so it stays readable when scrolled.
        let label = if drawn.waveform.is_some() {
            drawn.name.clone()
        } else {
            gettext("{} (audio file missing)").replace("{}", &drawn.name)
        };
        let layout = self.small_text(&label);
        // On a selected clip the name starts after the fade-in handle.
        let text_inset = if selected { 18.0 } else { 6.0 };
        layout.set_width(pango_units(
            box_right - box_left.max(0.0) - 2.0 * text_inset,
        ));
        layout.set_ellipsize(pango::EllipsizeMode::End);
        snapshot.save();
        snapshot.translate(&graphene::Point::new(
            box_left.max(0.0) + text_inset,
            top + 1.0,
        ));
        snapshot.append_layout(&layout, &palette.foreground);
        snapshot.restore();
        snapshot.pop();

        if selected {
            snapshot.append_border(&rounded, &[2.0; 4], &[palette.accent; 4]);
        }
    }

    /// A text layout in the "caption" size (a little smaller than normal text).
    fn small_text(&self, text: &str) -> pango::Layout {
        let layout = self.create_pango_layout(Some(text));
        let attributes = pango::AttrList::new();
        attributes.insert(pango::AttrFloat::new_scale(0.83));
        layout.set_attributes(Some(&attributes));
        layout
    }
}

/// Points along a fade curve from `x0` to `x1`: the gain from silent (at
/// `bottom`) to full (at `top`). A fade-in rises, a fade-out falls.
const FADE_STEPS: u32 = 32;

/// Shades what a fade removes and draws its curve.
/// Draws the playhead at `frame`: a 2-pixel line over the whole height.
fn draw_playhead(
    snapshot: &gtk::Snapshot,
    view: &View,
    palette: &Palette,
    frame: Frames,
    height: f32,
) {
    let x = view.x(frames_to_f64(frame));
    if (0.0..=f64::from(view.width)).contains(&x) {
        // Keep the 2-pixel line fully visible at the very start and end.
        let x = f32_from_f64(x).clamp(1.0, view.width - 1.0);
        snapshot.append_color(
            &palette.foreground,
            &graphene::Rect::new(x - 1.0, 0.0, 2.0, height),
        );
    }
}

/// Draws a time selection: a highlighted stretch over its lanes with a line at
/// each end, and a band on the ruler.
fn draw_time_selection(
    snapshot: &gtk::Snapshot,
    view: &View,
    palette: &Palette,
    selection: LaneRange,
) {
    if selection.range.is_empty() {
        return;
    }
    let left = f32_from_f64(view.x(frames_to_f64(selection.range.start)));
    let right = f32_from_f64(view.x(frames_to_f64(selection.range.end)));
    let top = lane_top(selection.first_lane);
    let bottom = lane_top(selection.last_lane + 1);
    let span = (right - left).max(1.0);
    snapshot.append_color(
        &palette.accent.with_alpha(0.18),
        &graphene::Rect::new(left, top, span, bottom - top),
    );
    snapshot.append_color(
        &palette.accent.with_alpha(0.3),
        &graphene::Rect::new(left, 0.0, span, pixels(RULER_HEIGHT)),
    );
    for x in [left, right] {
        snapshot.append_color(
            &palette.accent,
            &graphene::Rect::new(x - 0.5, top, 1.0, bottom - top),
        );
    }
}

/// A lane's clips as they would be if the drag ended now: the clips being moved
/// (`moved`, with the lane each is over) go on top, like on release.
fn lane_stack(clips: &[LaneClip], lane: usize, moved: &[(usize, Clip)]) -> Vec<Clip> {
    let is_moved = |id: ClipId| moved.iter().any(|(_, clip)| clip.id == id);
    clips
        .iter()
        .map(|clip| clip.clip)
        .filter(|clip| !is_moved(clip.id))
        .chain(
            moved
                .iter()
                .filter(|(over, _)| *over == lane)
                .map(|(_, clip)| *clip),
        )
        .collect()
}

/// A clip's fades: the part of the waveform a fade takes away is shaded, with
/// the fade's curve drawn on top. `edges` are the clip's left and right in
/// pixels, `wave` the top and bottom of its waveform area.
fn draw_clip_fades(
    snapshot: &gtk::Snapshot,
    view: &View,
    palette: &Palette,
    clip: &Clip,
    (left, right): (f32, f32),
    wave: (f32, f32),
) {
    let fade_width = |frames| f32_from_f64(frames_to_f64(frames) / view.frames_per_pixel);
    if clip.fade_in > 0 {
        let end = left + fade_width(clip.fade_in);
        draw_fade(snapshot, palette, (left, end), wave, true);
    }
    if clip.fade_out > 0 {
        let start = right - fade_width(clip.fade_out);
        draw_fade(snapshot, palette, (start, right), wave, false);
    }
}

fn draw_fade(
    snapshot: &gtk::Snapshot,
    palette: &Palette,
    (x0, x1): (f32, f32),
    (top, bottom): (f32, f32),
    rising: bool,
) {
    let curve: Vec<(f32, f32)> = (0..=FADE_STEPS)
        .map(|step| {
            let t = f64::from(step) / f64::from(FADE_STEPS);
            let gain = fade_shape(if rising { t } else { 1.0 - t });
            let x = x0 + (x1 - x0) * f32_from_f64(t);
            (x, bottom - f32_from_f64(gain) * (bottom - top))
        })
        .collect();
    let Some((&(first_x, first_y), rest)) = curve.split_first() else {
        return;
    };

    // The shaded area: above the curve, up to the top of the waveform area.
    let shade = gsk::PathBuilder::new();
    shade.move_to(first_x, top);
    for &(x, y) in &curve {
        shade.line_to(x, y);
    }
    shade.line_to(x1, top);
    shade.close();
    snapshot.append_fill(
        &shade.to_path(),
        gsk::FillRule::Winding,
        &palette.foreground.with_alpha(0.16),
    );

    let line = gsk::PathBuilder::new();
    line.move_to(first_x, first_y);
    for &(x, y) in rest {
        line.line_to(x, y);
    }
    snapshot.append_stroke(
        &line.to_path(),
        &gsk::Stroke::new(1.5),
        &palette.foreground.with_alpha(0.6),
    );
}

/// A round fade handle.
fn draw_handle(snapshot: &gtk::Snapshot, palette: &Palette, x: f32, y: f32) {
    let circle = gsk::PathBuilder::new();
    circle.add_circle(&graphene::Point::new(x, y), 5.0);
    let path = circle.to_path();
    snapshot.append_fill(&path, gsk::FillRule::Winding, &gdk::RGBA::WHITE);
    snapshot.append_stroke(&path, &gsk::Stroke::new(1.5), &palette.accent);
}

/// The outline of a waveform: along the maxima from left to right, then back
/// along the minima, scaled by `gain`. Column 0 is drawn at `first_x`. `None` if
/// no column has audio.
fn waveform_path(
    peaks: &[Option<Peak>],
    first_x: f32,
    middle: f32,
    half_height: f32,
    gain: f32,
) -> Option<gsk::Path> {
    let to_y = |value: f32| middle - (value * gain).clamp(-1.0, 1.0) * half_height * HEADROOM;
    // Every column is at least one pixel tall, so silence still shows as a line.
    let columns: Vec<(f32, f32, f32)> = peaks
        .iter()
        .enumerate()
        .filter_map(|(column, peak)| {
            let peak = (*peak)?;
            let x = first_x + index_f32(column) + 0.5;
            let (top, bottom) = (to_y(peak.max), to_y(peak.min));
            let grow = ((1.0 - (bottom - top)) / 2.0).max(0.0);
            Some((x, top - grow, bottom + grow))
        })
        .collect();
    let (&(first, first_top, _), rest) = columns.split_first()?;

    let builder = gsk::PathBuilder::new();
    builder.move_to(first, first_top);
    for &(x, top, _) in rest {
        builder.line_to(x, top);
    }
    for &(x, _, bottom) in columns.iter().rev() {
        builder.line_to(x, bottom);
    }
    builder.close();
    Some(builder.to_path())
}

/// The top of lane `index`, in pixels.
fn lane_top(index: usize) -> f32 {
    pixels(RULER_HEIGHT) + pixels(LANE_HEIGHT) * index_f32(index)
}

/// A widget size in pixels as `f32`, the unit GTK draws in.
fn pixels(value: i32) -> f32 {
    f32_from_f64(f64::from(value))
}

fn index_f32(index: usize) -> f32 {
    f32_from_f64(f64_from_usize(index))
}

/// Pixels to Pango units (Pango measures text in 1/1024ths of a pixel).
fn pango_units(pixels: f32) -> i32 {
    i32_from_f64(f64::from(pixels.max(0.0)) * 1024.0)
}

/// A block number from a fractional one (rounded down; negative counts as 0).
fn block_index(value: f64) -> usize {
    transient_core::time::frames_from_f64(value.max(0.0).floor())
        .and_then(|index| usize::try_from(index).ok())
        .unwrap_or(0)
}
