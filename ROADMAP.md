# Roadmap

Transient is built milestone by milestone; each one ends with something that
runs. Finished work is described in `CHANGELOG.md`. This file is the source of
truth for what comes next: update it when a milestone is finished or plans
change.

## Done

| # | Milestone | Commit |
|---|---|---|
| 0 | Project setup and an empty Adwaita window | `fda7462` |
| 1 | Import and play one audio file | `fda7462` |
| 2 | Waveform view with zoom, scrolling and seeking | `983b566` |
| 3 | Tracks and timeline, mixing several files | `9055854` |
| 4 | Editing (move, trim, split, delete, snapping) with undo/redo | `6330f9f` |
| 5 | Mixer: track volume, pan, mute, solo, master volume, level meter | `c9fa9ae` |
| 6 | Fades, clip gain, normalize (peak/LUFS), clip pane | `7210ccf` |
| 7 | Save and open projects, recent projects | `f603d76` |
| 8 | Export to WAV, FLAC, MP3, Opus, Ogg Vorbis | `33e85b2` |
| 9 | Polish: icon, preferences, Locate…, accessibility, MetaInfo | `b5f7479` |
| 10 | Cutting: time selection, ripple delete | `97272fd` |
| 11 | Tracks: new by dropping, rename, reorder, remove | `fb16a12` |
| 12 | Clipboard and multiple selection | `488f091` |
| 13 | Recording, with overdubbing and latency correction | `e5eaf9f` |
| 14 | Stricter Rust rules | `30a6aad` |

Next: milestone 15, Effects (below).

Already there from the original milestone 9 list: shortcuts dialog, About
dialog, dark mode (Libadwaita), error toasts.

## 9 · Polish ("usable every day")

- [x] **Touch and touchpad gestures** (requested): pinch to zoom
      (`GtkGestureZoom`, anchored at the point between the fingers), kinetic
      scrolling, dragging on a touchscreen. Two-finger sideways scroll and
      Ctrl+scroll zoom already work.
- [x] **Auto-scroll** while dragging a clip or clip edge near the edge of the view.
- [x] **Application icon.** Replace the placeholder
      (`data/icons/hicolor/scalable/apps/`) with an icon that follows the
      [GNOME HIG app icon guidelines](https://developer.gnome.org/hig/guidelines/app-icons.html):
      128×128 grid, simple shapes, the GNOME palette, a slight bottom edge for
      depth, readable at 32 px and 16 px. Take inspiration from the trans flag:
      light blue (#5BCEFA), pink (#F5A9B8) and white, matched to the nearest
      GNOME palette colours. Keep the waveform-with-a-transient idea. Update the
      symbolic icon (`hicolor/symbolic/apps/`) to match and check both in App
      Icon Preview.
- [x] **Preferences dialog** (Ctrl+,): the recording offset, and the format and
      quality the Export dialog starts with. The Export dialog remembers the
      last format exported to (`export-format`/`export-quality` settings), so
      Preferences only shows and changes that choice.
- [x] **Export a selection**: the export dialog offers Whole Project or
      Selection (the selected stretch on the selected tracks).
- [x] **"Locate…" for missing audio files** in the missing-files banner. Only
      projects saved before format 2 (and audio that was missing when saving)
      can still lack audio, so this matters much less now. The chosen file
      replaces the source (`Project::replace_source`, a new source ID, so undo
      makes it missing again); other missing files with the same name in that
      folder are used too. Scripted with `--locate=FILE`.
- [x] **Keyboard focus and accessibility pass**: the timeline is usable from
      the keyboard (arrows like a text cursor, Up/Down for tracks), every icon
      button has an accessible name, and the timeline describes and announces
      its state. Not yet checked with a real screen reader (Orca).
- [x] **Track colours** (requested 2026-11-05): the nine named Libadwaita colours
      (as in the accent picker in Settings) or "Default" (the accent colour),
      chosen in the track's menu; its clips are drawn in that colour.
- [x] **Light, dark or system style** (requested 2026-11-05): three circles at
      the top of the main menu, as in GNOME Text Editor; remembered.
- [x] **Release-ready MetaInfo**: a screenshot with a caption, a releases
      list, an OARS content rating, brand colours (trans flag pink, deep trans
      blue in dark mode) and a current description (see "Publishing" below).
- [x] **CI**: checked for flaky jobs (2026-11-05, the last 40 runs). One flaky
      failure: the Flatpak job's runtime update failed with "Invalid
      cross-device link" (run 41912431255), so both workflows now update the
      runtime first, with retries. Cancelled runs are superseded pushes
      (`cancel-in-progress`); the other failure was a real AppImage bug, fixed.

## 10 · Cutting

Inspired by Audacity 4's clip editing, kept GNOME-simple (no tool modes).

- [x] **Time selection** (decided 2026-11-04, like Audacity 4): drag over a
      clip's waveform or empty lane space to select time over the lanes dragged
      across; a clip's name strip moves it. Ctrl+A selects everything, Escape
      clears. Delete cuts the selection, leaving a gap (`transient-core::range`,
      property-tested).
- [x] **Ripple delete** (Ctrl+Delete, "Delete and Close Gap"): cuts the
      selection and moves everything after it left on all tracks; tracks outside
      the selection are moved, not cut.
- [x] **Split a selection**: S splits at both ends of the time selection (no
      separate split-at-the-mouse gesture: click, then press S).
- [x] **Show the selection's start, end and length** in the bottom bar.

## 11 · Tracks

Tracks stay (the mixer and ripple delete need them), but they should appear
when needed instead of being managed by hand.

- [x] **New track by dropping** a clip below the last track, or with Move to
      New Track in the clip pane (`Project::move_clip_to_new_track`, unit-tested).
- [x] **Import at the playhead** (decided 2026-11-04): every imported file
      still gets a new track, starting at the playhead instead of 0:00.
- [x] **Rename** (double-click the name, Rename… in the track menu, or F2) and
      **Move Up / Move Down** in the track menu (Alt+Up / Alt+Down).
- [x] **Reorder by dragging** a track header: a line shows where it lands,
      one undo step (`Project::move_track_to`, unit- and property-tested;
      `window/track_order.rs`; checked headlessly with a real drag).
- [x] **Remove Track** in the track menu (Undo toast, no confirmation). A track
      whose last clip is dragged to another track is removed automatically
      (decided 2026-11-04: tracks come from imported files).
- No mono/stereo track types: everything is stereo, and pan covers left/right.

## 12 · Clipboard and multiple selection

- [x] **Multiple selection**: Ctrl+click or Shift+click selects several clips; move, nudge and
      delete act on all of them.
- [x] **Cut, Copy, Paste** (Ctrl+X/C/V) for clips and time ranges, plus "Paste
      and Make Room" (Shift+Ctrl+V), which keeps tracks in sync.

## 14 · Stricter Rust rules (done, 2026-11-05)

Decided 2026-11-05: adopt the general-purpose parts of the Rust rules of
[FerroEHR](https://github.com/FerroHEALTH/FerroEHR) (`.claude/rules/rust-style.md`,
`comments.md`, `reliability.md` and its lint tables), which Transient's lint setup
was already based on. Comes before the open items of milestone 13.

Analysed 2026-11-05 (13,300 lines in 49 files). Already in line: no panics,
`Default`/serde, `#[expect]` with reasons, no import renames or re-exports, no block
comments or TODOs, comment runs within 8 lines, and every rustdoc lint is clean.
The work, in order:

- [x] **1. Lints.** Add to `[workspace.lints]`: `allow_attributes_without_reason`,
      `allow_attributes`, `too_long_first_doc_paragraph`, `panic_in_result_fn`,
      `missing_assert_message`, `string_slice`, `iter_over_hash_type`,
      `unchecked_time_subtraction`, `clone_on_ref_ptr`, `try_err`, `pub_use`,
      `trivial_casts`, `trivial_numeric_casts`, `unused_qualifications`,
      `redundant_lifetimes`, `dead_code = "deny"`, `pedantic` at deny, and a
      `[workspace.lints.rustdoc]` table. Findings: 9 doc comments whose first
      paragraph is too long (`archive.rs` ×2, `clipboard.rs`, `edit.rs`, `view.rs`
      ×3, `recorder.rs`) and the `#[allow]` on the tests in `waveform.rs` (becomes
      `#[expect]`). `draw.rs:338` has two doc comments on one function; keep one.
- [x] **2. Errors keep their cause.** 14 places turn an error into a string:
      `EngineError::Pipeline(String)` (fed by `From` impls for `BoolError` and
      `StateChangeError`, and by GStreamer error messages in `recorder.rs`,
      `player.rs`), `DecodeError::Unreadable(String)` (`decode.rs` ×4),
      `ExportError::Failed(String)` (`export.rs` ×2), and `Result<_, String>` across
      `spawn_blocking` in `recording.rs` ×2 and `file.rs`. They become variants with
      a `#[source]` (`io::Error`, `glib::Error`, `glib::BoolError`, …). Also fix the
      double prints: `ArchiveError::Io`/`Zip`, `ProjectError::Malformed` and
      `EngineError::Init` put their `#[from]` cause in their own message as well. One
      function prints an error with its causes (`a: b: c`) for logs and toasts.
- [x] **3. Number conversions without suppressions.** Production code has 13
      `as` conversions, each under an `#[expect]`, and the same helper written several
      times (`to_f32` in `mixer.rs`, `track_header.rs`, `clip_pane.rs`; `value as f32`
      in `draw.rs`, `mix.rs`, `normalize.rs`; `usize → f64` in `audio.rs` and
      `waveform.rs`). Plan: the `num-traits` crate (`ToPrimitive`/`NumCast`: checked
      conversions that return `Option`), wrapped in one module that owns them
      (`transient-core::convert`), with proptests; then `as_conversions` becomes deny.
      `u64 → f64` for frames can be exact without `as` (two `u32` halves). Tests keep
      their own `#[expect]`s.
- [x] **4. The other suppressions.** `too_many_arguments` on `draw_clip`: pass a
      struct with the drawing context. The `missing_docs` expect on
      `timeline_view/imp.rs` (generated by `glib::Properties`) and the `print_stderr`
      in `transient-cli` stay: one is inside a third-party macro, the other is what a
      command-line tool does. Both say so in their reason.
- [x] **5. Split `edit.rs`** (888 lines; at most 800, tests included) by concern:
      `clip.rs` (trim, split, fades, stacking: `covered_by`, `visible_parts`),
      `edit.rs` (changes to the project: move, place, tracks, snapping), `history.rs`
      (undo/redo). Add a file-size check to `just check` (a short bash script).
- [x] **6. Forwarders.** `Project::import` (calls `import_at(…, 0)`),
      `TimelineView::zoom_in`/`zoom_out` (call `zoom_by` with fixed values) and
      `Window::stop_recording_now` (calls `stop_recording`): callers call the target.
- [x] **7. Duplication.** Measured with jscpd 4.0.5 on 2026-11-05 (50 tokens, 5
      lines): 7 clones, 63 lines (0.5 %). Two are real duplicates: GStreamer time
      from frames (`export.rs` `time` and `player.rs` `frames_to_time`: one
      function in the engine) and "change the clip with this ID" in `clip_pane.rs`
      (normalize and fades: one helper). One is `use` lines (`export_dialog.rs`,
      `recording.rs`; not a duplicate). Four are test setup in `mix.rs`, `edit.rs`,
      `range.rs` and `clipboard.rs` (`render`, `two_tracks`); share those through a
      test-only module. With this little duplication, no permanent check.
- [x] **8. Write the rules down** in CLAUDE.md (Code rules), each with the tool that
      checks it.

Deliberately not adopted, because Transient is a desktop audio app and not a
clinical server: `integer_division` (15 places, all intended audio maths), the ban
on Python (`dev/send-keys.py` is a standalone dev script), the openEHR, HTTP, SQL
and time-library rules, and the CI ratchet workflows with count files. Two of his
lints cannot be met because the warnings come from `glib`'s macros
(`#[glib::object_subclass]`, `glib::Properties`), not our code: `same_name_method`
and `unused_lifetimes`.

## 13 · Recording (before 1.0, done 2026-11-05)

Requested 2026-11-04: recording comes before 1.0, not after.

- [x] **Record onto a new track** at the playhead (a Record button in the
      transport bar and R), with the level meter showing the input while
      recording. Stop (or Space) ends the take; it becomes an ordinary clip.
- [x] **Choose the input** (microphone, line in) from the arrow on the Record
      button; remembered in the settings. `transient-cli inputs` lists them.
- [x] **See the input's level before recording** (to set the microphone's
      volume): a live meter at the top of the Record button's input menu.
- [x] **Play the other tracks while recording** (overdubbing), so a voice can be
      recorded over music.
- [x] **Correct for the delay** between what plays and what is recorded
      (latency), so overdubs line up: takes are placed by the measured delay
      (`transient-core::take`), plus a "Recording Offset" in Preferences
      (decided 2026-11-05). Not yet checked with a real microphone and
      speakers. Later: a loopback assistant that measures the offset.
- [x] **Show the take growing** on the timeline while recording, with its
      outline, below the last track; the start page has a Record Audio button.
- [x] **Where takes are stored**: as FLAC in the project file, like imported
      audio (decided 2026-11-04).
- [x] **Permissions**: both Flatpak manifests have `--socket=pulseaudio` (there
      is no portal for the microphone yet). Explaining it to Flathub is in
      "Publishing".

## 15 · Effects (decided 2026-11-05)

A small, curated set of real-time effects with plain-language controls, so a
voice or band recording sounds finished without knowing audio engineering.
Non-destructive and saved in the project; written in Rust in `transient-core`
(unit-tested), since our own mixer renders the audio.

- **Per track**: low cut (rumble, handling noise), noise gate (hiss between
  phrases), compressor (evens out the volume), 3-band EQ (bass, middle, treble).
  **On the master**: a limiter (never clips).
- **Where**: a track pane, the right utility pane like the clip pane, opened with
  "Effects" in the track's menu. The master limiter sits with the master volume.
- **Controls**: each effect has an on/off switch and one slider in plain words
  (for example "Evenness" for the compressor); its real parameters (threshold,
  ratio, frequency, …) are in an expandable row below.
- **How the mix works with effects**: effects remember earlier audio (filters,
  envelopes), so `Mix` renders each track separately into its own buffer, runs
  that track's effect chain (before the track's volume and pan, as DAW inserts
  do), adds the tracks, then applies master volume and limiter. `render` takes
  `&mut self`. When playback jumps (a seek, or rendering a stretch that does not
  follow the previous one), the effects reset and run over a short stretch
  before the new position (a warm-up), so they start settled instead of with a
  click. After an edit, the new `Mix` takes over the effect state of the old one
  for tracks that still exist, so dragging a slider does not click.

Steps, each one runnable:

- [x] The mixer change: per-track rendering, `render(&mut self)`, warm-up on a
      jump, state carried over to the next `Mix` (`Mix::continue_from`, called
      by `Player::set_mix`). No audible change; all mix tests still pass.
- [x] Low cut end to end: the filter (`effects::biquad`, tested), the
      `effects` track field in the project file (format 3), the track pane
      (Effects in the track menu; `--action=track-effects` opens it for the
      selected or first track) with a switch, a Cut Below slider (logarithmic,
      20–500 Hz, its value shown beside it) and the slope (12/24/48 dB per
      octave, Butterworth). No Details row: with the value beside the slider
      and one other setting, nothing needed hiding (decided 2026-11-06). The range and default 24 dB slope came after
      a hands-on test (2026-11-06: 12 dB up to 300 Hz was hard to hear on
      laptop speakers, though measurably right).
- [x] An effects button (our own faders icon) in every track header, next
      to mute and solo: it opens the Track tab and is accent-coloured while
      an effect is on. Mute (warning colour) and solo (accent) are coloured
      while on too. Waveforms stay drawn from the
      original audio (as in Audacity and Ardour): a low cut barely changes a
      waveform's outline, a spectrogram would show it (see "Later").
- [x] Clip and Track tabs in the pane (Effects was hard to find in the track
      menu alone). The Track tab follows the selected clip's track.
- [x] Smooth gain changes during playback (found 2026-11-06, also in 0.3.0):
      a new clip gain, fade, volume, pan or master volume applies from one
      sample to the next, so a big change mid-playback clicks (a test: −12 dB
      on a 0.5 signal makes a step of 0.36, 25× the signal's largest normal
      step). Fixed: `Mix::continue_from` crossfades from the old mix to the
      new one over 20 ms, which covers every kind of change at once.
- [ ] Noise gate.
- [ ] Compressor.
- [ ] 3-band EQ.
- [ ] Master limiter.
- [ ] Screenshots, CHANGELOG, MetaInfo release notes.

## Publishing (1.0 and Flathub)

Checked against [Flathub's requirements](https://docs.flathub.org/docs/for-app-authors/requirements)
on 2026-11-04; check them again before submitting, the AI rules changed twice
in 2026.

**Flathub's rules on generative AI** (they apply because Transient is largely
written with an AI agent):

- Submitters must disclose AI-generated code, documentation and packaging:
  which parts, and roughly how much. Disclosure does not guarantee acceptance;
  reviewers may reject based on the amount of AI-generated material, code
  quality, or how reviewable and maintainable it is.
- The Flathub manifest must not contain any AI-generated or AI-assisted
  content, even with disclosure. `build-aux/*.Devel.json` was written by the
  agent, so it cannot be submitted.
- No AI or agent may open or automate the submission pull request, write its
  commit messages or description, or write replies to reviewers. AI-agent
  reviews must not be requested.
- Undisclosed or understated AI use can mean rejection, and repeated
  violations a permanent ban. Commit messages describe changes, not the tools
  used to make them, so the extent of AI use is stated in the submission
  itself (see "Draft the AI disclosure" below).

**To do before submitting** (the agent can help with these):

- [ ] **Development history**: Flathub wants sustained commits, tagged
      releases, real-world use and a commitment to maintenance. Tag releases
      regularly (0.1, 0.2, …) and use Transient for real work before 1.0.
- [x] **Project files hold their audio** (decided 2026-11-04, like Audacity's
      `.aup3`): a `.transient` file is a ZIP with the project and its audio as
      is, so the file chooser portal covers opening and saving.
- [ ] **Drop `--filesystem=home`** from the Flatpak. To do (by hand, later):
      remove the line from `build-aux/*.Devel.json`, reinstall the Flatpak
      (`just flatpak`), and check that these work without it:
      1. Open a project (Ctrl+O).
      2. Save, and Save As to a new place.
      3. Double-click a `.transient` file in Files.
      4. Recent projects on the start screen (most likely to fail: the saved
         paths may not be reachable without the permission).
      5. Import audio (Ctrl+I), then save.
      If all work, commit the removal; if not, keep the permission for now and
      look into the failing case.
- [ ] **Explain `--socket=pulseaudio`** in the Flathub submission: recording
      needs it, as there is no portal for the microphone yet; Flathub accepts
      it for audio apps.
- [x] **MetaInfo**: screenshots, a `<releases>` list, a content rating
      (OARS), brand colours; must pass validation (already checked by
      `meson test`). Before submitting: add a `<release>` for each new version,
      and check that the screenshot URL (on `main`) still shows the current app.
- [ ] **Translations** only from people: Flathub refuses machine-generated
      translations.
- [ ] **Runtime**: the latest GNOME runtime at the time of submitting.
- [x] **`RELEASING.md`** and a release workflow: a version tag builds a
      Flatpak bundle and an AppImage into a draft GitHub release.
- [x] **First test run of the release workflow**: both files build (run
      41901052553, 2026-11-04; the AppImage is about 120 MB).
- [ ] **Try the AppImage on a desktop** (by hand): icons, dark mode following
      the system, native Wayland, playback, recording, opening a project.
      Download it with `gh run download <run id> -n appimage`.
- [ ] **AppImage translations**: Transient looks for translations in an
      absolute folder (`/usr/share/locale`), which an AppImage cannot provide;
      once there are translations, look them up relative to the program.
- [ ] **Draft the AI disclosure** (a statement about the project; not part of
      the manifest or the pull request text).

**To do by the maintainer, without AI** (Flathub's rules):

- [ ] Write the Flathub manifest `io.github.sophiedkk.Transient.json`, with Rust
      crates as sources (generated by `flatpak-cargo-generator`, a regular
      tool) and no network access during the build.
- [ ] Open the submission pull request on `flathub/flathub` and handle the
      review conversation.

Already fine: the app ID (`io.github.` prefix, verified through the GitHub
account), the MIT licence, the SVG icon and the desktop file.

**If Flathub says no**: publish a `.flatpak` bundle with each GitHub release
(CI already builds one), or host our own Flatpak repository.

## Later (after the MVP)

- **More effects** (ideas from Audacity, 2026-11-05, after milestone 15; in
  order of value for effort):
  1. **Noise reduction** for voice recordings, Audacity's most-used effect.
     Needs a library choice first (for example an RNNoise port: check its
     licence and quality).
  2. **Volume automation**: points on a clip's volume line; automatic ducking
     of music under a voice track.
  3. **Export at a loudness target** (for example −16 LUFS for podcasts) for the
     whole mix; normalizing already measures loudness.
- Truncate silence (shorten long pauses), and a spectrogram view (repair work)
- Level meters per track, in the track headers (the master meter is under the
  master volume)
- Clip names and colours (a fixed set that works in light and dark themes)
- Markers and regions, and exporting one file per region
- GitHub issue-based workflow instead of this file (not yet)

Deliberately left out: changing a clip's speed or pitch by stretching it,
drawing individual samples, workspaces, plugin hosting (VST, LV2), macros and
batch processing: powerful, but they would turn Transient into another Audacity
instead of the simpler alternative.

## Open questions and known gaps

- Clips on one track stack (decided 2026-11-04, like Ardour): where they
  overlap, only the top one is seen and heard; nothing is cut. Fades on the
  top clip make a crossfade. A clip covered completely is hidden and cannot be
  clicked until the clip on top is moved away; it may need a marker, or a
  "Bring to Front"/"Send to Back" command.
- Mouse drags can be scripted headlessly since 2026-11-05 (`press`, `glide`,
  `release` in `dev/send-keys.py`), but only track reordering has been tested
  that way. Dragging clips, trim and fade handles, header sliders, spin rows and
  file choosers still need testing by hand.
- Waveforms show clip gain but not fades.
- Clicking in the timeline near or past the end of the audio during playback
  sometimes logs `GStreamer-CRITICAL: gst_segment_to_running_time: assertion
  'segment->format == format' failed` (seen 2026-11-06 in 0.3.0 too, timing-
  dependent). Playback carries on; find out which element complains.
- A fade's handle can cover the clip's name when the fade is short or the view
  zoomed out (e.g. a 2-second fade-in at the default zoom: "pi◯o"). Perhaps
  start the name after the handle, or draw the handle below the title bar.
