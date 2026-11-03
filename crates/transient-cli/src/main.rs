//! `transient-cli`: Transient's engine on the command line, without a window.
//!
//! Output is JSON (one object per line), so scripts and AI tools can read it.
//!
//! ```sh
//! transient-cli probe test-audio/*.ogg
//! transient-cli mix test-audio/drum-loop.wav test-audio/synth-loop.ogg
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::Context;
use clap::{Parser, Subcommand};
use serde::Serialize;
use transient_core::audio::{AudioBuffer, CHANNELS, to_dbfs};
use transient_core::mix::{AudioPool, Mix};
use transient_core::project::Project;
use transient_core::time::SampleRate;
use transient_engine::decode::decode_file;

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
    let mut project = Project::default();
    let mut pool = AudioPool::new();
    for file in files {
        let decoded = decode_file(file, project.sample_rate)
            .with_context(|| format!("decoding {}", file.display()))?;
        let name = file
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default();
        let track = project.import(file.clone(), decoded.buffer.frames(), name);
        let source = project
            .track(track)
            .and_then(|track| track.clips.first())
            .map(|clip| clip.source)
            .context("the imported clip is missing")?;
        pool.insert(source, decoded.buffer);
    }

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
            error: Some(error.to_string()),
            decoded: None,
        },
    }
}

fn round_db(db: f64) -> f64 {
    (db * 100.0).round() / 100.0
}
