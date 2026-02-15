//! retro-junk GUI
//!
//! Graphical user interface for analyzing retro game ROMs and disc images.
//! Uses MPSC channels to receive progress updates from analysis operations.

use std::sync::mpsc;

use retro_junk_lib::AnalysisProgress;

fn main() {
    println!("retro-junk GUI");
    println!("==============");
    println!();
    println!("GUI not yet implemented.");
    println!();
    println!("This application will use MPSC channels for progress updates:");

    // Demonstrate channel creation for progress updates
    let (tx, rx) = mpsc::channel::<AnalysisProgress>();

    // Example of how progress updates would be sent/received
    tx.send(AnalysisProgress::started(Some(1024 * 1024)))
        .unwrap();
    tx.send(AnalysisProgress::phase("Reading header")).unwrap();
    tx.send(AnalysisProgress::reading(512, Some(1024 * 1024)))
        .unwrap();
    tx.send(AnalysisProgress::found("Serial: SLUS-00123"))
        .unwrap();
    tx.send(AnalysisProgress::Completed).unwrap();

    println!();
    println!("Progress update examples:");
    while let Ok(progress) = rx.try_recv() {
        match progress {
            AnalysisProgress::Started { total_bytes } => {
                println!(
                    "  Started: {} bytes",
                    total_bytes.map_or("unknown".to_string(), |b| b.to_string())
                );
            }
            AnalysisProgress::Phase {
                name,
                current,
                total,
            } => {
                if let (Some(c), Some(t)) = (current, total) {
                    println!("  Phase {}/{}: {}", c, t, name);
                } else {
                    println!("  Phase: {}", name);
                }
            }
            AnalysisProgress::Reading {
                bytes_read,
                total_bytes,
            } => {
                if let Some(total) = total_bytes {
                    let pct = (bytes_read as f64 / total as f64) * 100.0;
                    println!("  Reading: {}/{} bytes ({:.1}%)", bytes_read, total, pct);
                } else {
                    println!("  Reading: {} bytes", bytes_read);
                }
            }
            AnalysisProgress::Found { description } => {
                println!("  Found: {}", description);
            }
            AnalysisProgress::Completed => {
                println!("  Completed!");
            }
            AnalysisProgress::Failed { message } => {
                println!("  Failed: {}", message);
            }
        }
    }
}
