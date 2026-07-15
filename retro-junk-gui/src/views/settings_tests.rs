//! D1: the chdman probe must run off the UI thread, not block a frame.
//!
//! Uses a fake "chdman" shell script that sleeps briefly before exiting
//! non-zero (never prints the CHD banner, so the probe always resolves to
//! `Err`) — the same fake-chdman-script technique
//! `retro-junk-lib/src/tests/chd_convert_tests.rs` uses for its B2 timing
//! tests. The sleep gives a comfortable window to prove a frame renders and
//! returns long before the probe could have completed synchronously.

use egui_kittest::Harness;

use crate::app::RetroJunkApp;

/// Write a slow fake "chdman": sleeps, then exits non-zero without ever
/// printing the CHD banner `Chdman::detect` looks for.
#[cfg(unix)]
fn write_slow_fake_chdman(dir: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let path = dir.join("slow-fake-chdman.sh");
    std::fs::write(&path, "#!/bin/sh\nsleep 0.3\nexit 1\n").unwrap();
    let mut perms = std::fs::metadata(&path).unwrap().permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&path, perms).unwrap();
    path
}

#[cfg(unix)]
fn harness_with_slow_chdman<'a>(chdman_path: std::path::PathBuf) -> Harness<'a, RetroJunkApp> {
    Harness::new_eframe(move |cc| {
        let mut settings = crate::settings::AppSettings::default();
        settings.general.chdman_path = chdman_path.display().to_string();
        let mut app = RetroJunkApp::with_parts(&cc.egui_ctx, settings, None, None);
        app.current_view = crate::state::View::Settings;
        app
    })
}

#[test]
#[cfg(unix)]
fn chdman_probe_runs_off_the_ui_thread() {
    let dir = tempfile::TempDir::new().unwrap();
    let chdman_path = write_slow_fake_chdman(dir.path());
    let mut harness = harness_with_slow_chdman(chdman_path);

    // The probe binary sleeps 300ms before ever responding. If the probe ran
    // synchronously in-frame, this call would take at least that long; it
    // must instead return promptly, well under the sleep duration.
    let start = std::time::Instant::now();
    harness.step();
    let frame_time = start.elapsed();
    assert!(
        frame_time < std::time::Duration::from_millis(150),
        "rendering the settings view took {frame_time:?} — the chdman probe appears to be \
         running synchronously on the UI thread instead of in the background"
    );

    assert!(
        harness.state().chdman_probe_in_flight,
        "expected a probe to be kicked off for the configured path"
    );
    assert!(
        harness.state().chdman_probe.is_none(),
        "the probe result must not appear until the background thread's message is delivered"
    );

    // Drain frames until the background thread's result message lands.
    let mut settled = false;
    for _ in 0..100 {
        std::thread::sleep(std::time::Duration::from_millis(20));
        harness.step();
        if !harness.state().chdman_probe_in_flight {
            settled = true;
            break;
        }
    }
    assert!(settled, "chdman probe never completed");
    assert!(harness.state().chdman_probe.is_some());
    // The fake binary never prints the CHD banner, so this must be an error.
    assert!(harness.state().chdman_probe.as_ref().unwrap().1.is_err());
}
