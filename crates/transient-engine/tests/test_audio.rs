//! Decodes every file in the test audio collection (dev/test-audio.tsv).
//!
//! The files are downloaded, not committed, so these tests are `#[ignore]`d by
//! default. Run them with `just test-audio`, which downloads the files first.

use std::path::PathBuf;

use transient_core::audio::to_dbfs;
use transient_core::time::SampleRate;
use transient_engine::decode::decode_file;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The file names listed in dev/test-audio.tsv.
fn test_files() -> Vec<String> {
    // A missing manifest gives an empty list, which the test's file count check reports.
    let manifest =
        std::fs::read_to_string(repo_root().join("dev/test-audio.tsv")).unwrap_or_default();
    manifest
        .lines()
        .filter(|line| !line.starts_with('#') && !line.trim().is_empty())
        .filter_map(|line| line.split('\t').next())
        .map(str::to_owned)
        .collect()
}

#[test]
#[ignore = "needs the downloaded test audio: run `just test-audio`"]
fn every_test_file_decodes_to_audible_stereo_at_48_khz() {
    transient_engine::init().unwrap();
    let files = test_files();
    assert!(files.len() >= 8, "the manifest lists {} files", files.len());

    for name in files {
        let path = repo_root().join("test-audio").join(&name);
        let decoded = decode_file(&path, SampleRate::HZ_48000)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        let buffer = decoded.buffer;

        assert_eq!(buffer.sample_rate(), SampleRate::HZ_48000, "{name}");
        assert!(buffer.duration_seconds() > 5.0, "{name} is too short");
        let peak = to_dbfs(buffer.peak()).unwrap_or(f64::NEG_INFINITY);
        assert!(
            peak > -30.0,
            "{name} is nearly silent (peak {peak:.1} dBFS)"
        );
        assert!(decoded.source_format.is_some(), "{name}: no source format");
    }
}
