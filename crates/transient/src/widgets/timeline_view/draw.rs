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
use transient_core::mix::{db_to_gain, fade_shape};
use transient_core::project::Clip;
use transient_core::ruler;
use transient_core::time::frames_to_f64;
use transient_core::waveform::Peak;

use super::{CLIP_INSET, CLIP_LABEL_HEIGHT, LANE_HEIGHT, LaneClip, RULER_HEIGHT, TimelineView};

/// Labelled ruler ticks are at least this far apart, in pixels.
const TICK_SPACING: f64 = 90.0;
/// Rounded corners of clips.
const CLIP_RADIUS: f32 = 6.0;
/// Clips of tracks that are not heard (muted, or another track is soloed).
const SILENT_LANE_OPACITY: f64 = 0.4;
/// The waveform uses this much of its half-height, leaving a little room.
const HEADROOM: f32 = 0.92;

/// The colours used, taken from the current theme.
struct Palette {
    foreground: gdk::RGBA,
    accent: gdk::RGBA,
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
        };
        let imp = self.imp();
        let view = View {
            width,
            start: self.start_frame(),
            frames_per_pixel: imp.frames_per_pixel.get(),
        };

        self.draw_ruler(snapshot, &view, &palette);

        // A clip being dragged is drawn at its preview position, on top of the rest.
        let drag = imp.drag.borrow();
        let preview = drag.as_ref().and_then(|drag| drag.preview);
        let selected = imp.selected.get();
        let lanes = imp.lanes.borrow();
        let mut dragged = None;
        for (index, lane) in lanes.iter().enumerate() {
            let top = lane_top(index);
            snapshot.append_color(
                &palette.foreground.with_alpha(0.08),
                &graphene::Rect::new(0.0, top + pixels(LANE_HEIGHT) - 1.0, width, 1.0),
            );
            if !lane.audible {
                snapshot.push_opacity(SILENT_LANE_OPACITY);
            }
            for clip in &lane.clips {
                if preview.is_some_and(|(_, moved)| moved.id == clip.clip.id) {
                    dragged = Some(clip);
                    continue;
                }
                let is_selected = selected == Some(clip.clip.id);
                self.draw_clip(
                    snapshot,
                    &view,
                    &palette,
                    clip,
                    &clip.clip,
                    top,
                    is_selected,
                );
            }
            if !lane.audible {
                snapshot.pop();
            }
        }
        if let (Some(clip), Some((lane, moved))) = (dragged, preview) {
            self.draw_clip(
                snapshot,
                &view,
                &palette,
                clip,
                &moved,
                lane_top(lane),
                true,
            );
        }

        if self.rate().is_some() {
            let x = view.x(frames_to_f64(imp.playhead.get()));
            if (0.0..=f64::from(width)).contains(&x) {
                // Keep the 2-pixel line fully visible at the very start and end.
                let x = pixels_f64(x).clamp(1.0, width - 1.0);
                snapshot.append_color(
                    &palette.foreground,
                    &graphene::Rect::new(x - 1.0, 0.0, 2.0, height),
                );
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
            let x = pixels_f64(tick.x);
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

    /// One clip: a rounded box with its name and waveform. `clip` is where to draw
    /// it (its own position, or a drag preview); `drawn` supplies waveform and name.
    #[expect(
        clippy::too_many_arguments,
        reason = "drawing state is passed explicitly rather than hidden in a struct"
    )]
    fn draw_clip(
        &self,
        snapshot: &gtk::Snapshot,
        view: &View,
        palette: &Palette,
        drawn: &LaneClip,
        clip: &Clip,
        lane_top: f32,
        selected: bool,
    ) {
        let left = view.x(frames_to_f64(clip.start));
        let right = view.x(frames_to_f64(clip.end()));
        if right < 0.0 || left > f64::from(view.width) {
            return;
        }
        // Clamp to just outside the view: GTK draws huge rectangles badly.
        let (box_left, box_right) = (
            pixels_f64(left.max(-f64::from(CLIP_RADIUS))),
            pixels_f64(right.min(f64::from(view.width) + f64::from(CLIP_RADIUS))),
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
        let columns = usize::try_from(column_count(last_column - first_column)).unwrap_or(0);
        let source_start =
            frames_to_f64(clip.offset) + view.start + first_column * view.frames_per_pixel
                - frames_to_f64(clip.start);
        let peaks = drawn.waveform.as_ref().map_or_else(Vec::new, |waveform| {
            waveform.peaks_within(
                clip.offset..clip.offset.saturating_add(clip.length),
                source_start,
                view.frames_per_pixel,
                columns,
            )
        });
        // Drawn with the clip's gain applied, so normalizing visibly changes it.
        if let Some(path) = waveform_path(
            &peaks,
            pixels_f64(first_column),
            wave_top + half_height,
            half_height,
            db_to_gain(clip.gain_db),
        ) {
            snapshot.append_fill(&path, gsk::FillRule::Winding, &palette.accent);
        }

        // Fades: the part of the waveform a fade takes away is shaded, with the
        // fade's curve drawn on top.
        let bottom = top + height;
        let fade_width = |frames| pixels_f64(frames_to_f64(frames) / view.frames_per_pixel);
        let (clip_left, clip_right) = (pixels_f64(left), pixels_f64(right));
        if clip.fade_in > 0 {
            let end = clip_left + fade_width(clip.fade_in);
            draw_fade(
                snapshot,
                palette,
                (clip_left, end),
                (wave_top, bottom),
                true,
            );
        }
        if clip.fade_out > 0 {
            let start = clip_right - fade_width(clip.fade_out);
            draw_fade(
                snapshot,
                palette,
                (start, clip_right),
                (wave_top, bottom),
                false,
            );
        }

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
            // Fade handles, dragged to change the fades (see interaction.rs).
            let (fade_in_x, fade_out_x) = self.fade_handle_positions(clip);
            let y = top + CLIP_LABEL_HEIGHT / 2.0;
            for x in [fade_in_x, fade_out_x] {
                draw_handle(snapshot, palette, pixels_f64(x), y);
            }
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
            let x = x0 + (x1 - x0) * pixels_f64(t);
            (x, bottom - pixels_f64(gain) * (bottom - top))
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
    pixels_f64(f64::from(value))
}

fn index_f32(index: usize) -> f32 {
    pixels_f64(f64::from(u32::try_from(index).unwrap_or(u32::MAX)))
}

fn pixels_f64(value: f64) -> f32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "pixel coordinates are small; f32 is what GTK draws with"
    )]
    let value = value as f32;
    value
}

/// A whole number of columns (the input is already rounded and non-negative).
fn column_count(columns: f64) -> u32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a column count is a small whole number; `as` saturates out-of-range values"
    )]
    let columns = columns.max(0.0) as u32;
    columns
}

/// Pixels to Pango units (Pango measures text in 1/1024ths of a pixel).
fn pango_units(pixels: f32) -> i32 {
    #[expect(
        clippy::as_conversions,
        clippy::cast_possible_truncation,
        reason = "text widths are small; `as` saturates out-of-range values"
    )]
    let units = (pixels.max(0.0) * 1024.0) as i32;
    units
}
