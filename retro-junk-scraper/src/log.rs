use std::path::Path;

use crate::lookup::LookupMethod;

/// A single entry in the scrape log.
#[derive(Debug, Clone)]
pub enum LogEntry {
    Success {
        file: String,
        game_name: String,
        method: LookupMethod,
        media_downloaded: Vec<String>,
    },
    Partial {
        file: String,
        game_name: String,
        warnings: Vec<String>,
    },
    Unidentified {
        file: String,
        serial_tried: Option<String>,
        filename_tried: bool,
        hashes_tried: bool,
        errors: Vec<String>,
    },
    Error {
        file: String,
        message: String,
    },
}

/// Collects scrape results and writes a log file.
#[derive(Debug, Default)]
pub struct ScrapeLog {
    entries: Vec<LogEntry>,
}

impl ScrapeLog {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add(&mut self, entry: LogEntry) {
        self.entries.push(entry);
    }

    pub fn summary(&self) -> LogSummary {
        let mut summary = LogSummary::default();
        for entry in &self.entries {
            match entry {
                LogEntry::Success { method, media_downloaded, .. } => {
                    summary.total_success += 1;
                    summary.media_downloaded += media_downloaded.len();
                    match method {
                        LookupMethod::Serial => summary.by_serial += 1,
                        LookupMethod::Filename => summary.by_filename += 1,
                        LookupMethod::Hash => summary.by_hash += 1,
                    }
                }
                LogEntry::Partial { .. } => summary.total_partial += 1,
                LogEntry::Unidentified { .. } => summary.total_unidentified += 1,
                LogEntry::Error { .. } => summary.total_errors += 1,
            }
        }
        summary
    }

    /// Write the log to a file.
    pub fn write_to_file(&self, path: &Path) -> std::io::Result<()> {
        use std::io::Write;

        let mut file = std::fs::File::create(path)?;
        let summary = self.summary();

        writeln!(file, "=== Scrape Log ===")?;
        writeln!(file, "Date: {}", chrono::Local::now().format("%Y-%m-%d %H:%M:%S"))?;
        writeln!(file)?;
        writeln!(file, "--- Summary ---")?;
        writeln!(file, "Successful: {} (serial: {}, filename: {}, hash: {})",
            summary.total_success, summary.by_serial, summary.by_filename, summary.by_hash)?;
        writeln!(file, "Partial: {}", summary.total_partial)?;
        writeln!(file, "Unidentified: {}", summary.total_unidentified)?;
        writeln!(file, "Errors: {}", summary.total_errors)?;
        writeln!(file, "Media downloaded: {}", summary.media_downloaded)?;
        writeln!(file)?;
        writeln!(file, "--- Details ---")?;
        writeln!(file)?;

        for entry in &self.entries {
            match entry {
                LogEntry::Success { file: f, game_name, method, media_downloaded } => {
                    writeln!(file, "[OK] {} -> \"{}\" (matched by {})", f, game_name, method)?;
                    if !media_downloaded.is_empty() {
                        writeln!(file, "     Media: {}", media_downloaded.join(", "))?;
                    }
                }
                LogEntry::Partial { file: f, game_name, warnings } => {
                    writeln!(file, "[PARTIAL] {} -> \"{}\"", f, game_name)?;
                    for w in warnings {
                        writeln!(file, "     Warning: {}", w)?;
                    }
                }
                LogEntry::Unidentified { file: f, serial_tried, filename_tried, hashes_tried, errors } => {
                    writeln!(file, "[UNIDENTIFIED] {}", f)?;
                    if let Some(serial) = serial_tried {
                        writeln!(file, "     Serial tried: {}", serial)?;
                    }
                    if *filename_tried {
                        writeln!(file, "     Filename lookup: tried")?;
                    }
                    if *hashes_tried {
                        writeln!(file, "     Hash lookup: tried")?;
                    }
                    for e in errors {
                        writeln!(file, "     Error: {}", e)?;
                    }
                }
                LogEntry::Error { file: f, message } => {
                    writeln!(file, "[ERROR] {}: {}", f, message)?;
                }
            }
        }

        Ok(())
    }
}

#[derive(Debug, Default)]
pub struct LogSummary {
    pub total_success: usize,
    pub total_partial: usize,
    pub total_unidentified: usize,
    pub total_errors: usize,
    pub media_downloaded: usize,
    pub by_serial: usize,
    pub by_filename: usize,
    pub by_hash: usize,
}
