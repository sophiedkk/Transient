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

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::time::{Frames, SampleRate};

/// The project file format version written by this build of Transient.
///
/// Raise it whenever a change means older versions of Transient could
/// misread a new file. Files with a *higher* number are refused when opened.
pub const FORMAT_VERSION: u32 = 1;

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

/// A part of a source, placed on the timeline.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
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
}

impl Clip {
    /// The first frame on the timeline *after* the clip.
    #[must_use]
    pub fn end(&self) -> Frames {
        self.start.saturating_add(self.length)
    }
}

/// A row in the timeline, holding clips.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Track {
    /// Identifies the track.
    pub id: TrackId,
    /// The name shown in the track header.
    pub name: String,
    /// The clips, in no particular order.
    pub clips: Vec<Clip>,
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
            next_id: 1,
        }
    }
}

/// Why a project could not be read or written.
#[derive(Debug, Error)]
pub enum ProjectError {
    /// The file is not valid JSON, or does not have the expected structure.
    #[error("the project file is damaged: {0}")]
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

    /// Adds an empty track at the bottom.
    pub fn add_track(&mut self, name: String) -> TrackId {
        let id = TrackId(self.new_id());
        self.tracks.push(Track {
            id,
            name,
            clips: Vec::new(),
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
            offset: 0,
            length,
        });
        Some(id)
    }

    /// Imports a file the usual way: a new track named `name`, with the whole
    /// file as one clip at the start of the timeline.
    pub fn import(&mut self, path: PathBuf, frames: Frames, name: String) -> TrackId {
        let source = self.add_source(path, frames);
        let track = self.add_track(name);
        // Both were just created, so this cannot fail.
        let _clip = self.add_clip(track, source, 0);
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
        project.import("/music/drums.wav".into(), 96_000, "drums".to_owned());
        let json = project.to_json().unwrap();
        insta::assert_snapshot!(json, @r#"
        {
          "format_version": 1,
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
                  "length": 96000
                }
              ]
            }
          ],
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
        project.import("intro.flac".into(), 1000, "intro".to_owned());
        let reopened = Project::from_json(&project.to_json().unwrap()).unwrap();
        assert_eq!(reopened, project);
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
    fn the_timeline_ends_with_the_last_clip() {
        let mut project = Project::default();
        assert_eq!(project.length(), 0);
        let track = project.import("a.wav".into(), 1000, "a".to_owned());
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
}
