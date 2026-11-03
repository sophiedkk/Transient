//! `transient-cli`: Transient's engine on the command line, without a window.
//!
//! Output is JSON (one object per line), so scripts and AI tools can read it.
//!
//! ```sh
//! transient-cli probe test-audio/*.ogg
//! ```

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Instant;

use anyhow::Context;
use clap::{Parser, Subcommand};
use serde::Serialize;
use transient_core::audio::to_dbfs;
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
    }
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
