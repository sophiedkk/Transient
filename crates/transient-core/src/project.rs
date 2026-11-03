//! The project: everything that is saved in a Transient project file.
//!
//! Milestone 0 only holds the basics; tracks and clips are added in milestone 3.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::time::SampleRate;

/// The project file format version written by this build of Transient.
///
/// Raise it whenever a change means older versions of Transient could
/// misread a new file. Files with a *higher* number are refused when opened.
pub const FORMAT_VERSION: u32 = 1;

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
}

impl Default for Project {
    fn default() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            name: String::new(),
            sample_rate: SampleRate::default(),
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A snapshot test: if the file format changes, this test fails and shows the
    /// difference, so a format change is always a deliberate decision.
    #[test]
    fn default_project_file() {
        let json = Project::default().to_json().unwrap();
        insta::assert_snapshot!(json, @r#"
        {
          "format_version": 1,
          "name": "",
          "sample_rate": 48000
        }
        "#);
    }

    #[test]
    fn save_and_open_gives_the_same_project() {
        let project = Project {
            name: "Podcast episode 1".to_owned(),
            sample_rate: SampleRate::new(44_100).unwrap(),
            ..Project::default()
        };
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
}
