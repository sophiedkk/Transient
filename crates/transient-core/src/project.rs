//! The project: everything that is saved in a Transient project file.
//!
//! A project has *sources* (imported audio files) and *tracks*. Each track holds
//! *clips*: a clip plays part of a source at a position on the timeline.
//!
//! ```text
//! source  |-------------------- song.flac --------------------|
//!                 offset ↓     length
//! clip                   |==============|
//! timeline   0 ······ start ↑
//! ```
//!
//! Editing never changes the audio itself, only these numbers. The file stores
//! the sources' paths; the decoded audio lives in memory (see [`crate::mix`]).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::time::{Frames, SampleRate};

/// The project file format version written by this build of Transient.
///
/// Raise it whenever a change means older versions of Transient could
/// misread a new file. Files with a *higher* number are refused when opened.
///
/// - 1: a plain JSON file, with paths to the audio files.
/// - 2: a ZIP archive holding this JSON (`project.json`) and the audio
///   (see [`crate::archive`]); source paths point into the archive.
pub const FORMAT_VERSION: u32 = 2;

/// Defines a numeric ID type. Separate types for sources, tracks and clips mean
/// the compiler refuses to mix them up.
macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        #[derive(
            Debug, Clone, Copy, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub u64);
    };
}

id_type!(
    /// Identifies a [`Source`] within a project.
    SourceId
);
id_type!(
    /// Identifies a [`Track`] within a project.
    TrackId
);
id_type!(
    /// Identifies a [`Clip`] within a project.
    ClipId
);

/// An imported audio file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Source {
    /// Identifies the source; clips refer to it by this ID.
    pub id: SourceId,
    /// Where the file is.
    pub path: PathBuf,
    /// The length of the decoded audio, at the project sample rate.
    pub frames: Frames,
}

/// A part of a source, placed on the timeline, with its own gain and fades.
///
/// The derived `Default` is also the right start for gain and fades: 0 dB,
/// no fades.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Clip {
    /// Identifies the clip.
    pub id: ClipId,
    /// The audio it plays.
    pub source: SourceId,
    /// Where it starts on the timeline.
    pub start: Frames,
    /// Where in the source it starts playing (the "in-point").
    pub offset: Frames,
    /// How long it plays.
    pub length: Frames,
    /// The clip's own volume in decibels (set by normalizing, for example).
    pub gain_db: f32,
    /// How long it takes to fade in from silence at the start.
    pub fade_in: Frames,
    /// How long it takes to fade out to silence at the end.
    pub fade_out: Frames,
}

impl Clip {
    /// The first frame on the timeline *after* the clip.
    #[must_use]
    pub fn end(&self) -> Frames {
        self.start.saturating_add(self.length)
    }
}

/// A row in the timeline, holding clips, with its mixer settings.
///
/// The derived `Default` is also the right starting point for the mixer
/// settings: 0 dB (unchanged), centred, not muted, not soloed.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    /// Identifies the track.
    pub id: TrackId,
    /// The name shown in the track header.
    pub name: String,
    /// The clips, in no particular order.
    pub clips: Vec<Clip>,
    /// Volume in decibels: 0 is unchanged, −6 about half, at or below
    /// [`crate::mix::SILENT_DB`] silent.
    pub volume_db: f32,
    /// Balance from −1 (left only) through 0 (centre) to 1 (right only).
    pub pan: f32,
    /// A muted track is not heard.
    pub mute: bool,
    /// When any track is soloed, only soloed tracks are heard.
    pub solo: bool,
    /// The colour its clips are drawn in; `None` is the system's accent colour.
    pub color: Option<TrackColor>,
}

/// A track's colour: one of the named colours of the GNOME desktop.
///
/// They are the colours of the accent colour picker in Settings. The app draws
/// each in the theme's shade of it (lighter in dark mode), so no colour value is
/// stored here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TrackColor {
    /// Blue.
    Blue,
    /// Teal.
    Teal,
    /// Green.
    Green,
    /// Yellow.
    Yellow,
    /// Orange.
    Orange,
    /// Red.
    Red,
    /// Pink.
    Pink,
    /// Purple.
    Purple,
    /// Slate (a blue-grey).
    Slate,
}

impl TrackColor {
    /// Every colour, in the order the picker shows them.
    pub const ALL: [Self; 9] = [
        Self::Blue,
        Self::Teal,
        Self::Green,
        Self::Yellow,
        Self::Orange,
        Self::Red,
        Self::Pink,
        Self::Purple,
        Self::Slate,
    ];

    /// The colour's name as stored in project files ("blue", "teal", …), also
    /// used as the colour menu's action target.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Blue => "blue",
            Self::Teal => "teal",
            Self::Green => "green",
            Self::Yellow => "yellow",
            Self::Orange => "orange",
            Self::Red => "red",
            Self::Pink => "pink",
            Self::Purple => "purple",
            Self::Slate => "slate",
        }
    }

    /// The colour with this [`TrackColor::id`], if there is one.
    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|color| color.id() == id)
    }
}

/// A Transient project.
///
/// `#[serde(default)]` fills every field missing from a file with its value from
/// [`Project::default`], so files from older versions keep opening.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Project {
    /// Which version of the file format this project was saved with.
    pub format_version: u32,
    /// The name shown in the window title. Empty means "not named yet".
    pub name: String,
    /// The rate every source file is converted to when it is imported.
    pub sample_rate: SampleRate,
    /// The imported audio files.
    pub sources: Vec<Source>,
    /// The tracks, top to bottom.
    pub tracks: Vec<Track>,
    /// The volume of the whole mix, in decibels.
    pub master_volume_db: f32,
    /// The next unused ID (shared by sources, tracks and clips).
    pub next_id: u64,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            name: String::new(),
            sample_rate: SampleRate::default(),
            sources: Vec::new(),
            tracks: Vec::new(),
            master_volume_db: 0.0,
            next_id: 1,
        }
    }
}

/// Why a project could not be read or written.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// The file is not valid JSON, or does not have the expected structure.
    #[error("the project file is damaged")]
    Malformed(#[from] serde_json::Error),
    /// The file was saved by a newer version of Transient.
    #[error(
        "the project was saved by a newer version of Transient (format {found}, supported up to {supported})"
    )]
    TooNew {
        /// The format version found in the file.
        found: u32,
        /// The newest format version this build understands.
        supported: u32,
    },
}

impl Project {
    /// Reads a project from the contents of a project file.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectError::Malformed`] if `json` is not a valid project, and
    /// [`ProjectError::TooNew`] if it was saved by a newer version of Transient.
    pub fn from_json(json: &str) -> Result<Self, ProjectError> {
        let project: Self = serde_json::from_str(json)?;
        if project.format_version > FORMAT_VERSION {
            return Err(ProjectError::TooNew {
                found: project.format_version,
                supported: FORMAT_VERSION,
            });
        }
        Ok(project)
    }

    /// Writes the project as human-readable JSON.
    ///
    /// # Errors
    ///
    /// Returns [`ProjectError::Malformed`] if the project cannot be serialized.
    pub fn to_json(&self) -> Result<String, ProjectError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// The project as it is saved in `folder`: sources inside that folder (or a
    /// folder in it) get a path relative to it, so the project keeps working
    /// when the whole folder is moved or copied. Other paths stay absolute.
    #[must_use]
    pub fn with_paths_relative_to(&self, folder: &Path) -> Self {
        let mut project = self.clone();
        for source in &mut project.sources {
            if let Ok(relative) = source.path.strip_prefix(folder) {
                source.path = relative.to_owned();
            }
        }
        project
    }

    /// The project as opened from `folder`: relative source paths are turned
    /// back into full paths. The opposite of [`Self::with_paths_relative_to`].
    #[must_use]
    pub fn with_paths_resolved_from(&self, folder: &Path) -> Self {
        let mut project = self.clone();
        for source in &mut project.sources {
            if source.path.is_relative() {
                source.path = folder.join(&source.path);
            }
        }
        project
    }

    /// Hands out a fresh ID number.
    fn new_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        id
    }

    /// Registers an imported file of `frames` length.
    pub fn add_source(&mut self, path: PathBuf, frames: Frames) -> SourceId {
        let id = SourceId(self.new_id());
        self.sources.push(Source { id, path, frames });
        id
    }

    /// Points the clips of source `old` at another file, `frames` long (for a
    /// file that was missing and has been found elsewhere). The file gets a new
    /// source in `old`'s place, with a new ID, so decoded audio kept by ID
    /// always belongs to the path it came from: undoing brings `old` back,
    /// still missing. Returns the new ID, or `None` if `old` does not exist.
    pub fn replace_source(
        &mut self,
        old: SourceId,
        path: PathBuf,
        frames: Frames,
    ) -> Option<SourceId> {
        let index = self.sources.iter().position(|source| source.id == old)?;
        let id = SourceId(self.new_id());
        let source = self.sources.get_mut(index)?;
        *source = Source { id, path, frames };
        for clip in self.tracks.iter_mut().flat_map(|track| &mut track.clips) {
            if clip.source == old {
                clip.source = id;
            }
        }
        Some(id)
    }

    /// Adds an empty track at the bottom.
    pub fn add_track(&mut self, name: String) -> TrackId {
        let id = TrackId(self.new_id());
        self.tracks.push(Track {
            id,
            name,
            ..Track::default()
        });
        id
    }

    /// Places the whole of `source` on `track`, starting at `start`.
    /// Returns `None` if the track or source does not exist.
    pub fn add_clip(&mut self, track: TrackId, source: SourceId, start: Frames) -> Option<ClipId> {
        let length = self.source(source)?.frames;
        let id = ClipId(self.new_id());
        self.track_mut(track)?.clips.push(Clip {
            id,
            source,
            start,
            length,
            ..Clip::default()
        });
        Some(id)
    }

    /// Imports a file: a new track named `name`, with the whole file (`frames`
    /// long) as one clip starting at `start` (the app imports at the playhead).
    pub fn import(
        &mut self,
        path: PathBuf,
        frames: Frames,
        name: String,
        start: Frames,
    ) -> TrackId {
        let source = self.add_source(path, frames);
        let track = self.add_track(name);
        // Both were just created, so this cannot fail.
        let _clip = self.add_clip(track, source, start);
        track
    }

    /// The source with this ID.
    #[must_use]
    pub fn source(&self, id: SourceId) -> Option<&Source> {
        self.sources.iter().find(|source| source.id == id)
    }

    /// The track with this ID.
    #[must_use]
    pub fn track(&self, id: TrackId) -> Option<&Track> {
        self.tracks.iter().find(|track| track.id == id)
    }

    /// The track with this ID, for changing it.
    pub fn track_mut(&mut self, id: TrackId) -> Option<&mut Track> {
        self.tracks.iter_mut().find(|track| track.id == id)
    }

    /// The length of the timeline: up to the end of the last clip.
    #[must_use]
    pub fn length(&self) -> Frames {
        self.tracks
            .iter()
            .flat_map(|track| &track.clips)
            .map(Clip::end)
            .max()
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot test: if the file format changes, this test fails and shows the
    /// difference, so a format change is always a deliberate decision.
    #[test]
    fn project_file_format() {
        let mut project = Project::default();
        project.import("/music/drums.wav".into(), 96_000, "drums".to_owned(), 0);
        let json = project.to_json().unwrap();
        insta::assert_snapshot!(json, @r#"
        {
          "format_version": 2,
          "name": "",
          "sample_rate": 48000,
          "sources": [
            {
              "id": 1,
              "path": "/music/drums.wav",
              "frames": 96000
            }
          ],
          "tracks": [
            {
              "id": 2,
              "name": "drums",
              "clips": [
                {
                  "id": 3,
                  "source": 1,
                  "start": 0,
                  "offset": 0,
                  "length": 96000,
                  "gain_db": 0.0,
                  "fade_in": 0,
                  "fade_out": 0
                }
              ],
              "volume_db": 0.0,
              "pan": 0.0,
              "mute": false,
              "solo": false,
              "color": null
            }
          ],
          "master_volume_db": 0.0,
          "next_id": 4
        }
        "#);
    }

    #[test]
    fn save_and_open_gives_the_same_project() {
        let mut project = Project {
            name: "Podcast episode 1".to_owned(),
            sample_rate: SampleRate::new(44_100).unwrap(),
            ..Project::default()
        };
        project.import("intro.flac".into(), 1000, "intro".to_owned(), 0);
        let reopened = Project::from_json(&project.to_json().unwrap()).unwrap();
        assert_eq!(reopened, project);
    }

    #[test]
    fn a_track_colour_is_saved_by_name_and_is_optional() {
        let mut project = Project::default();
        let track = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        if let Some(track) = project.track_mut(track) {
            track.color = Some(TrackColor::Teal);
        }
        let json = project.to_json().unwrap();
        assert!(json.contains(r#""color": "teal""#), "{json}");
        assert_eq!(Project::from_json(&json).unwrap(), project);
        // A track from an older file has no colour: the accent colour.
        let old = Project::from_json(r#"{ "tracks": [{ "id": 1, "name": "a" }] }"#).unwrap();
        assert_eq!(old.tracks[0].color, None);
    }

    #[test]
    fn every_colour_has_its_own_id() {
        for color in TrackColor::ALL {
            assert_eq!(TrackColor::from_id(color.id()), Some(color));
        }
        assert_eq!(TrackColor::from_id("magenta"), None);
    }

    #[test]
    fn missing_fields_get_their_defaults() {
        assert_eq!(Project::from_json("{}").unwrap(), Project::default());
    }

    #[test]
    fn files_from_newer_versions_are_refused() {
        let error = Project::from_json(r#"{ "format_version": 999 }"#).unwrap_err();
        assert!(matches!(error, ProjectError::TooNew { found: 999, .. }));
    }

    #[test]
    fn a_sample_rate_of_zero_is_refused() {
        let error = Project::from_json(r#"{ "sample_rate": 0 }"#).unwrap_err();
        assert!(matches!(error, ProjectError::Malformed(_)));
    }

    #[test]
    fn sources_inside_the_project_folder_get_relative_paths() {
        let mut project = Project::default();
        project.add_source("/music/mix/stems/drums.wav".into(), 10);
        project.add_source("/elsewhere/vocals.wav".into(), 10);

        let saved = project.with_paths_relative_to(Path::new("/music/mix"));
        assert_eq!(saved.sources[0].path, Path::new("stems/drums.wav"));
        assert_eq!(saved.sources[1].path, Path::new("/elsewhere/vocals.wav"));

        // Opened from a moved folder, the relative path follows the folder.
        let opened = saved.with_paths_resolved_from(Path::new("/backup/mix"));
        assert_eq!(
            opened.sources[0].path,
            Path::new("/backup/mix/stems/drums.wav")
        );
        assert_eq!(opened.sources[1].path, Path::new("/elsewhere/vocals.wav"));
    }

    #[test]
    fn the_timeline_ends_with_the_last_clip() {
        let mut project = Project::default();
        assert_eq!(project.length(), 0);
        let track = project.import("a.wav".into(), 1000, "a".to_owned(), 0);
        let source = project.add_source("b.wav".into(), 500);
        project.add_clip(track, source, 2000).unwrap();
        assert_eq!(project.length(), 2500);
    }

    #[test]
    fn clips_need_an_existing_track_and_source() {
        let mut project = Project::default();
        let track = project.add_track("empty".to_owned());
        let source = project.add_source("a.wav".into(), 10);
        assert_eq!(project.add_clip(TrackId(999), source, 0), None);
        assert_eq!(project.add_clip(track, SourceId(999), 0), None);
    }

    #[test]
    fn a_found_file_replaces_the_missing_source_for_its_clips() {
        let mut project = Project::default();
        let track = project.import("/old/a.wav".into(), 1000, "a".to_owned(), 0);
        let old = project.sources[0].id;
        let other = project.add_source("/old/b.wav".into(), 10);
        project.add_clip(track, old, 5000).unwrap();
        project.add_clip(track, other, 9000).unwrap();

        let new = project
            .replace_source(old, "/new/a.wav".into(), 1200)
            .unwrap();
        assert_ne!(new, old);
        // Same place in the list, new path and length; the other source stays.
        assert_eq!(
            project.sources[0],
            Source {
                id: new,
                path: "/new/a.wav".into(),
                frames: 1200
            }
        );
        assert_eq!(project.sources[1].id, other);
        let clips: Vec<SourceId> = project.tracks[0]
            .clips
            .iter()
            .map(|clip| clip.source)
            .collect();
        assert_eq!(clips, [new, new, other]);
        assert_eq!(
            project.replace_source(SourceId(999), "x.wav".into(), 1),
            None
        );
    }
}
