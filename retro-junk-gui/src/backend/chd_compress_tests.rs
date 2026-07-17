//! D3: `start_compression` must no-op when a compression is already running
//! for the target console folder, instead of launching a second overlapping
//! run against the same inputs/outputs.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use retro_junk_lib::chd_convert::{Chdman, CompressionJob};

use super::start_compression;
use crate::app::RetroJunkApp;
use crate::state::{
    BackgroundOperation, ChdCompressItem, ChdCompressPrompt, OperationKind, ProgressDisplay,
};

fn fake_job(input_bytes: u64) -> CompressionJob {
    CompressionJob {
        input: PathBuf::from("/roms/psx/Game.cue"),
        media: retro_junk_core::ChdMedia::Cd,
        output: PathBuf::from("/roms/psx/Game.chd"),
        source_files: vec![PathBuf::from("/roms/psx/Game.cue")],
        input_bytes,
    }
}

#[test]
fn start_compression_noops_when_busy() {
    let mut app = RetroJunkApp::with_parts(
        &egui::Context::default(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );

    // A ChdCompress op is already scoped to folder "psx".
    app.operations.push(BackgroundOperation::new(
        1,
        "existing compression".to_string(),
        Arc::new(AtomicBool::new(false)),
        OperationKind::ChdCompress,
        "psx".to_string(),
        ProgressDisplay::Percent,
    ));

    app.chd_compress_prompt = Some(ChdCompressPrompt {
        folder_name: "psx".to_string(),
        items: vec![ChdCompressItem {
            job: fake_job(100),
            display_line: "Game.cue (100 B) \u{2192} Game.chd".to_string(),
        }],
        skipped: Vec::new(),
        chdman: Ok(Chdman {
            path: PathBuf::from("chdman"),
            version: String::new(),
        }),
        delete_sources: false,
    });

    let ctx = egui::Context::default();
    start_compression(&mut app, &ctx);

    // The prompt is always consumed (taken), but no second ChdCompress op
    // was spawned — still just the one pre-existing operation.
    assert!(app.chd_compress_prompt.is_none());
    assert_eq!(
        app.operations.len(),
        1,
        "start_compression must not spawn a second run while one is busy for this folder"
    );
}

#[test]
fn start_compression_spawns_when_not_busy() {
    let mut app = RetroJunkApp::with_parts(
        &egui::Context::default(),
        crate::settings::AppSettings::default(),
        None,
        None,
    );

    app.chd_compress_prompt = Some(ChdCompressPrompt {
        folder_name: "psx".to_string(),
        items: vec![ChdCompressItem {
            job: fake_job(100),
            display_line: "Game.cue (100 B) \u{2192} Game.chd".to_string(),
        }],
        skipped: Vec::new(),
        // A bogus chdman path: compress_to_chd will fail fast inside the
        // worker thread (spawn error), which is fine — this test only
        // checks that the operation gets registered, not that it succeeds.
        chdman: Ok(Chdman {
            path: PathBuf::from("/nonexistent/chdman-binary"),
            version: String::new(),
        }),
        delete_sources: false,
    });

    let ctx = egui::Context::default();
    start_compression(&mut app, &ctx);

    assert!(app.chd_compress_prompt.is_none());
    assert_eq!(
        app.operations.len(),
        1,
        "a compression op should be spawned"
    );
    assert_eq!(app.operations[0].kind, OperationKind::ChdCompress);
    assert_eq!(app.operations[0].scope, "psx");

    // Let the worker thread finish and clean up so the test doesn't leak a
    // background thread past the end of the test process.
    if let Some(handle) = app.op_threads.remove(&app.operations[0].id) {
        let _ = handle.join();
    }
}
