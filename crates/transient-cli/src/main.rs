//! `transient-cli`: Transient's engine on the command line, without a window.
//!
//! Output is JSON (one object per line), so scripts and AI tools can read it.
//!
//! ```sh
//! transient-cli probe test-audio/*.ogg
//! transient-cli mix test-audio/drum-loop.wav test-audio/synth-loop.ogg
//! transient-cli export my-mix.transient --output my-mix.flac
//! transient-cli export test-audio/*.wav --output both.mp3
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;
use std::time::Instant;

use anyhow::Context;
use clap::{Parser, Subcommand};
use serde::Serialize;
use transient_core::audio::{AudioBuffer, CHANNELS, to_dbfs};
use transient_core::mix::{AudioPool, Mix};
use transient_core::project::Project;
use transient_core::report::Report;
use transient_core::time::SampleRate;

use transient_engine::decode::decode_file;
use transient_engine::export::{self, ExportFormat};

#[derive(Debug, Parser)]
#[command(version, about = "Transient's audio engine on the command line")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Decode files exactly like the app does, and report what came out.
    Probe {
        /// Audio files to decode.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Put each file on its own track (like importing them in the app), mix
    /// everything, and report the result.
    Mix {
        /// Audio files to mix.
        #[arg(required = true)]
        files: Vec<PathBuf>,
    },
    /// Export a saved project (or audio files, one track each) to a file. The
    /// format follows the output's extension: wav, flac, mp3, opus or ogg.
    Export {
        /// A .transient project, or audio files.
        #[arg(required = true)]
        input: Vec<PathBuf>,
        /// The file to write.
        #[arg(short, long)]
        output: PathBuf,
    },
    /// List the sound inputs (microphones, line inputs) the app can record
    /// from, one JSON line each, with the ID the app remembers them by.
    Inputs,
}

/// One line of `inputs`.
#[derive(Debug, Serialize)]
struct InputReport {
    name: String,
    id: String,
}

/// The output of `export`.
#[derive(Debug, Serialize)]
struct ExportReport {
    output: PathBuf,
    format: String,
    seconds: f64,
    export_ms: u128,
}

/// The output of `mix`.
#[derive(Debug, Serialize)]
struct MixReport {
    tracks: usize,
    frames: u64,
    seconds: f64,
    peak_dbfs: Option<f64>,
    rms_dbfs: Option<f64>,
    render_ms: u128,
}

/// One line of `probe` output.
#[derive(Debug, Serialize)]
struct ProbeReport {
    file: PathBuf,
    ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decoded: Option<DecodedReport>,
}

#[derive(Debug, Serialize)]
struct DecodedReport {
    frames: u64,
    seconds: f64,
    sample_rate: u32,
    source_sample_rate: Option<u32>,
    source_channels: Option<u32>,
    peak_dbfs: Option<f64>,
    rms_dbfs: Option<f64>,
    decode_ms: u128,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::from(1),
        Err(error) => {
            // Writing to stderr is this tool's job, so the lint does not apply here.
            #[expect(clippy::print_stderr, reason = "a CLI reports fatal errors on stderr")]
            {
                eprintln!("error: {error:#}");
            }
            ExitCode::from(2)
        }
    }
}

/// Returns `Ok(false)` if some files failed (they are reported in the output).
fn run(cli: Cli) -> anyhow::Result<bool> {
    transient_engine::init().context("starting GStreamer")?;
    match cli.command {
        Command::Probe { files } => {
            let mut stdout = std::io::stdout().lock();
            let mut all_ok = true;
            for file in files {
                let report = probe(&file);
                all_ok &= report.ok;
                serde_json::to_writer(&mut stdout, &report)?;
                writeln!(stdout)?;
            }
            Ok(all_ok)
        }
        Command::Export { input, output } => {
            let report = export(&input, &output)?;
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer(&mut stdout, &report)?;
            writeln!(stdout)?;
            Ok(true)
        }
        Command::Inputs => {
            let mut stdout = std::io::stdout().lock();
            for device in transient_engine::recorder::inputs() {
                let report = InputReport {
                    id: device.id(),
                    name: device.name,
                };
                serde_json::to_writer(&mut stdout, &report)?;
                writeln!(stdout)?;
            }
            Ok(true)
        }
        Command::Mix { files } => {
            let report = mix(&files)?;
            let mut stdout = std::io::stdout().lock();
            serde_json::to_writer(&mut stdout, &report)?;
            writeln!(stdout)?;
            Ok(true)
        }
    }
}

/// Builds a project the way the app does on import, then renders the whole mix
/// in chunks, the way the player does.
fn mix(files: &[PathBuf]) -> anyhow::Result<MixReport> {
    let (project, pool) = import_files(files)?;
    let started = Instant::now();
    let mix = Mix::new(&project, &pool);
    let mut rendered = Vec::new();
    let mut chunk = vec![0.0; 4096 * CHANNELS];
    let mut position = 0;
    while position < mix.length() {
        mix.render(position, &mut chunk);
        let frames = (mix.length() - position).min(4096);
        let samples = usize::try_from(frames)? * CHANNELS;
        rendered.extend_from_slice(chunk.get(..samples).unwrap_or_default());
        position += frames;
    }
    let render_ms = started.elapsed().as_millis();
    let result = AudioBuffer::new(mix.sample_rate(), rendered)?;

    Ok(MixReport {
        tracks: project.tracks.len(),
        frames: result.frames(),
        seconds: (result.duration_seconds() * 1000.0).round() / 1000.0,
        peak_dbfs: to_dbfs(result.peak()).map(round_db),
        rms_dbfs: to_dbfs(result.rms()).map(round_db),
        render_ms,
    })
}

/// Exports a project file, or audio files on one track each.
fn export(input: &[PathBuf], output: &Path) -> anyhow::Result<ExportReport> {
    let extension = output
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or_default();
    let format = ExportFormat::from_extension(extension)
        .with_context(|| format!("unknown export format “.{extension}”"))?;
    let (project, pool) = match input {
        [file] if file.extension().is_some_and(|ext| ext == "transient") => open_project(file)?,
        files => import_files(files)?,
    };
    let mix = Mix::new(&project, &pool);
    let started = Instant::now();
    export::export(&mix, output, format, |_| {}, &AtomicBool::new(false))
        .with_context(|| format!("exporting to {}", output.display()))?;
    Ok(ExportReport {
        output: output.to_owned(),
        format: format!("{format:?}"),
        seconds: (mix.sample_rate().frames_to_seconds(mix.length()) * 1000.0).round() / 1000.0,
        export_ms: started.elapsed().as_millis(),
    })
}

/// Opens a saved project and decodes its sources, like the app does. The audio
/// is unpacked into a temporary folder, deleted again when the decoding is done.
fn open_project(file: &Path) -> anyhow::Result<(Project, AudioPool)> {
    let cache = tempfile::tempdir().context("creating a folder for the project's audio")?;
    let file = std::path::absolute(file)?;
    let project = transient_core::archive::open(&file, cache.path())
        .with_context(|| format!("reading {}", file.display()))?;
    let mut pool = AudioPool::new();
    for source in &project.sources {
        let decoded = decode_file(&source.path, project.sample_rate)
            .with_context(|| format!("decoding {}", source.path.display()))?;
        pool.insert(source.id, decoded.buffer);
    }
    Ok((project, pool))
}

/// Puts each file on its own track, like importing them in the app.
fn import_files(files: &[PathBuf]) -> anyhow::Result<(Project, AudioPool)> {
    let mut project = Project::default();
    let mut pool = AudioPool::new();
    for file in files {
        let decoded = decode_file(file, project.sample_rate)
            .with_context(|| format!("decoding {}", file.display()))?;
        let name = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let track = project.import(file.clone(), decoded.buffer.frames(), name, 0);
        let source = project
            .track(track)
            .and_then(|track| track.clips.first())
            .map(|clip| clip.source)
            .context("the imported clip is missing")?;
        pool.insert(source, decoded.buffer);
    }
    Ok((project, pool))
}

fn probe(file: &Path) -> ProbeReport {
    let started = Instant::now();
    match decode_file(file, SampleRate::default()) {
        Ok(decoded) => {
            let buffer = &decoded.buffer;
            ProbeReport {
                file: file.to_owned(),
                ok: true,
                error: None,
                decoded: Some(DecodedReport {
                    frames: buffer.frames(),
                    seconds: (buffer.duration_seconds() * 1000.0).round() / 1000.0,
                    sample_rate: buffer.sample_rate().hz(),
                    source_sample_rate: decoded.source_format.map(|f| f.sample_rate),
                    source_channels: decoded.source_format.map(|f| f.channels),
                    peak_dbfs: to_dbfs(buffer.peak()).map(round_db),
                    rms_dbfs: to_dbfs(buffer.rms()).map(round_db),
                    decode_ms: started.elapsed().as_millis(),
                }),
            }
        }
        Err(error) => ProbeReport {
            file: file.to_owned(),
            ok: false,
            error: Some(Report(&error).to_string()),
            decoded: None,
        },
    }
}

fn round_db(db: f64) -> f64 {
    (db * 100.0).round() / 100.0
}
