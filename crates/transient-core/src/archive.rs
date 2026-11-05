//! Project files on disk. A `.transient` file is a ZIP archive with:
//!
//! - `project.json`: the [`Project`] (tracks, clips, mixer settings);
//! - `audio/`: a copy of every audio file the project uses, exactly as it was
//!   imported (an MP3 stays an MP3), as `audio/<source id>/<file name>`.
//!
//! So a project never loses its audio, and it can be moved, backed up or shared
//! as one file. (Audacity's `.aup3` works the same way, but stores raw samples.)
//!
//! Opening unpacks the audio into a cache folder and points the sources there.
//! Saving writes a new archive next to the old one and then renames it over
//! it, so a failed save never leaves a damaged project behind.
//!
//! Format version 1 files were plain JSON with paths to audio files elsewhere
//! (relative to the project's folder when inside it); they still open, and get
//! their audio on the next save.

use std::fs::File;
use std::io::{self, BufReader, BufWriter, Read, Seek, Write};
use std::path::{Path, PathBuf};

use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipArchive, ZipWriter};

use crate::project::{Project, ProjectError, SourceId};

/// The project inside the archive.
const PROJECT_ENTRY: &str = "project.json";
/// The folder inside the archive that holds the audio.
const AUDIO_FOLDER: &str = "audio/";

/// Why a project file could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum ArchiveError {
    /// Reading or writing the file failed.
    #[error("could not read or write the file")]
    Io(#[from] io::Error),
    /// The archive itself is damaged.
    #[error("not a valid project file")]
    Zip(#[from] zip::result::ZipError),
    /// The project inside it is damaged or too new.
    #[error(transparent)]
    Project(#[from] ProjectError),
}

/// Writes `project` to `path`, with a copy of each source's audio.
///
/// The audio is read from each source's `path`. A source whose file cannot be
/// read keeps pointing at that path instead (it was missing already); the result
/// says which ones.
///
/// # Errors
///
/// Returns [`ArchiveError`] if the file cannot be written; `path` is then left
/// as it was.
pub fn save(project: &Project, path: &Path) -> Result<Vec<SourceId>, ArchiveError> {
    let folder = path
        .parent()
        .filter(|folder| !folder.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    // Written next to the destination, then renamed over it: never half a file.
    let temporary = tempfile::NamedTempFile::new_in(folder)?;
    let mut zip = ZipWriter::new(BufWriter::new(temporary.reopen()?));

    let mut saved = project.clone();
    let mut not_embedded = Vec::new();
    // Everything is stored uncompressed: audio is compressed already (or a WAV,
    // which barely shrinks), and project.json is small.
    let stored = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Stored)
        .large_file(true);
    for source in &mut saved.sources {
        let Ok(mut audio) = File::open(&source.path) else {
            not_embedded.push(source.id);
            continue;
        };
        let entry = audio_entry_name(source.id, &source.path);
        zip.start_file(entry.as_str(), stored)?;
        io::copy(&mut audio, &mut zip)?;
        source.path = PathBuf::from(entry);
    }

    zip.start_file(PROJECT_ENTRY, stored)?;
    zip.write_all(saved.to_json()?.as_bytes())?;
    let mut writer = zip.finish()?;
    writer.flush()?;
    drop(writer);
    temporary
        .persist(path)
        .map_err(|error| ArchiveError::Io(error.error))?;
    Ok(not_embedded)
}

/// Reads the project at `path`.
///
/// Its audio is unpacked into `cache` (a folder for this project, created if
/// needed) and the sources point there. An old plain-JSON project (format 1) is
/// read as it was, with paths resolved from its folder.
///
/// # Errors
///
/// Returns [`ArchiveError`] if the file cannot be read or is not a project.
pub fn open(path: &Path, cache: &Path) -> Result<Project, ArchiveError> {
    let mut file = BufReader::new(File::open(path)?);
    if !is_zip(&mut file)? {
        let mut json = String::new();
        file.read_to_string(&mut json)?;
        let folder = path.parent().unwrap_or_else(|| Path::new("."));
        return Ok(Project::from_json(&json)?.with_paths_resolved_from(folder));
    }

    let mut zip = ZipArchive::new(file)?;
    let mut json = String::new();
    zip.by_name(PROJECT_ENTRY)?.read_to_string(&mut json)?;
    let mut project = Project::from_json(&json)?;
    std::fs::create_dir_all(cache)?;
    for source in &mut project.sources {
        let Some(entry) = source
            .path
            .to_str()
            .filter(|name| name.starts_with(AUDIO_FOLDER))
        else {
            // Not in the archive (it could not be read when saving): a path.
            continue;
        };
        let Ok(mut audio) = zip.by_name(entry) else {
            // Listed but not there: the clips show as missing audio.
            continue;
        };
        // "audio/3/piano.flac" becomes "<cache>/3/piano.flac": the file keeps
        // its name (clips are labelled with it), the folder keeps sources apart.
        let unpacked = cache.join(entry.trim_start_matches(AUDIO_FOLDER));
        if let Some(folder) = unpacked.parent() {
            std::fs::create_dir_all(folder)?;
        }
        io::copy(&mut audio, &mut BufWriter::new(File::create(&unpacked)?))?;
        source.path = unpacked;
    }
    Ok(project)
}

/// Whether `file` starts like a ZIP archive ("PK\x03\x04"). Leaves the read
/// position at the start.
fn is_zip(file: &mut (impl Read + Seek)) -> io::Result<bool> {
    let mut magic = [0_u8; 4];
    let read = file.read(&mut magic)?;
    file.rewind()?;
    Ok(read == 4 && magic == *b"PK\x03\x04")
}

/// The name of a source's audio in the archive: `audio/<id>/<file name>`. The
/// ID keeps two files with the same name apart.
fn audio_entry_name(id: SourceId, path: &Path) -> String {
    let name = path.file_name().map_or_else(
        || "audio".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );
    format!("{AUDIO_FOLDER}{}/{name}", id.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project with two sources: real files with known bytes in `folder`.
    fn project_with_audio(folder: &Path) -> Project {
        let mut project = Project::default();
        for (name, bytes) in [
            ("drums.wav", &b"RIFF drums"[..]),
            ("voice.mp3", &b"ID3 voice"[..]),
        ] {
            let path = folder.join(name);
            std::fs::write(&path, bytes).unwrap();
            project.import(path, 1000, name.to_owned(), 0);
        }
        project
    }

    #[test]
    fn a_saved_project_carries_its_audio() {
        let work = tempfile::tempdir().unwrap();
        let project = project_with_audio(work.path());
        let file = work.path().join("song.transient");
        assert!(save(&project, &file).unwrap().is_empty());

        // The originals can go: the project still has the audio.
        std::fs::remove_file(work.path().join("drums.wav")).unwrap();
        let cache = work.path().join("cache");
        let opened = open(&file, &cache).unwrap();
        assert_eq!(opened.tracks, project.tracks);
        let drums = &opened.sources[0].path;
        assert!(drums.starts_with(&cache));
        assert_eq!(std::fs::read(drums).unwrap(), b"RIFF drums");
        // Unpacked with its own name, so clips keep their labels.
        assert_eq!(opened.sources[1].path.file_name().unwrap(), "voice.mp3");
    }

    #[test]
    fn a_source_that_cannot_be_read_keeps_its_path() {
        let work = tempfile::tempdir().unwrap();
        let mut project = project_with_audio(work.path());
        project.sources[1].path = work.path().join("gone.wav");
        let file = work.path().join("song.transient");
        let missing = save(&project, &file).unwrap();
        assert_eq!(missing, vec![project.sources[1].id]);
        let opened = open(&file, &work.path().join("cache")).unwrap();
        assert_eq!(opened.sources[1].path, work.path().join("gone.wav"));
    }

    #[test]
    fn saving_again_replaces_the_file() {
        let work = tempfile::tempdir().unwrap();
        let mut project = project_with_audio(work.path());
        let file = work.path().join("song.transient");
        save(&project, &file).unwrap();
        project.name = "second".to_owned();
        save(&project, &file).unwrap();
        let opened = open(&file, &work.path().join("cache")).unwrap();
        assert_eq!(opened.name, "second");
    }

    #[test]
    fn an_old_json_project_still_opens() {
        let work = tempfile::tempdir().unwrap();
        let project = project_with_audio(work.path());
        let old = project.with_paths_relative_to(work.path());
        let json = old.to_json().unwrap().replace(
            &format!("\"format_version\": {}", crate::project::FORMAT_VERSION),
            "\"format_version\": 1",
        );
        let file = work.path().join("old.transient");
        std::fs::write(&file, json).unwrap();
        let opened = open(&file, &work.path().join("cache")).unwrap();
        assert_eq!(opened.sources[0].path, work.path().join("drums.wav"));
    }

    #[test]
    fn something_else_is_refused() {
        let work = tempfile::tempdir().unwrap();
        let file = work.path().join("notes.transient");
        std::fs::write(&file, "shopping list").unwrap();
        assert!(open(&file, &work.path().join("cache")).is_err());
    }
}
