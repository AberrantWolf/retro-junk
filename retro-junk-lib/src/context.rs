//! Analysis context for ROM analysis.

use std::path::{Path, PathBuf};

use retro_junk_core::{Platform, RomAnalyzer};

/// Metadata about a registered console.
#[derive(Debug, Clone)]
pub struct Console {
    /// Platform identifier
    pub platform: Platform,
    /// Short name (e.g., "snes", "n64")
    pub short_name: &'static str,
    /// Full platform name
    pub platform_name: &'static str,
    /// Manufacturer name
    pub manufacturer: &'static str,
    /// File extensions
    pub extensions: &'static [&'static str],
    /// Alternative folder names
    pub folder_names: &'static [&'static str],
}

impl Console {
    /// Create console metadata from an analyzer.
    pub fn from_analyzer<A: RomAnalyzer>(analyzer: &A) -> Self {
        Self {
            platform: analyzer.platform(),
            short_name: analyzer.short_name(),
            platform_name: analyzer.platform_name(),
            manufacturer: analyzer.manufacturer(),
            extensions: analyzer.file_extensions(),
            folder_names: analyzer.folder_names(),
        }
    }
}

/// A registered console with its analyzer.
pub struct RegisteredConsole {
    pub metadata: Console,
    pub analyzer: Box<dyn RomAnalyzer>,
}

impl RegisteredConsole {
    pub fn new<A: RomAnalyzer + 'static>(analyzer: A) -> Self {
        let metadata = Console::from_analyzer(&analyzer);
        Self {
            metadata,
            analyzer: Box::new(analyzer),
        }
    }
}

/// Context holding all registered console analyzers.
///
/// This is the main entry point for using the library. Create a context,
/// register consoles, then use it to analyze ROMs.
pub struct AnalysisContext {
    consoles: Vec<RegisteredConsole>,
}

impl Default for AnalysisContext {
    fn default() -> Self {
        Self::new()
    }
}

impl AnalysisContext {
    /// Create a new empty context.
    pub fn new() -> Self {
        Self {
            consoles: Vec::new(),
        }
    }

    /// Register a console analyzer.
    pub fn register<A: RomAnalyzer + 'static>(&mut self, analyzer: A) -> &mut Self {
        self.consoles.push(RegisteredConsole::new(analyzer));
        self
    }

    /// Get all registered consoles.
    pub fn consoles(&self) -> impl Iterator<Item = &RegisteredConsole> {
        self.consoles.iter()
    }

    /// Get a console by its `Platform` enum variant.
    pub fn get_by_platform(&self, platform: Platform) -> Option<&RegisteredConsole> {
        self.consoles
            .iter()
            .find(|c| c.metadata.platform == platform)
    }

    /// Get a console by short name or alias.
    pub fn get_by_short_name(&self, short_name: &str) -> Option<&RegisteredConsole> {
        if let Ok(platform) = short_name.parse::<Platform>() {
            self.get_by_platform(platform)
        } else {
            None
        }
    }

    /// Find consoles that match a folder name.
    pub fn find_by_folder(&self, folder_name: &str) -> Vec<&RegisteredConsole> {
        self.consoles
            .iter()
            .filter(|c| c.analyzer.matches_folder(folder_name))
            .collect()
    }

    /// List all short names.
    pub fn short_names(&self) -> Vec<&'static str> {
        self.consoles
            .iter()
            .map(|c| c.metadata.short_name)
            .collect()
    }

    /// Check if a folder name matches any registered console.
    pub fn matches_any_console(&self, folder_name: &str) -> bool {
        self.consoles
            .iter()
            .any(|c| c.analyzer.matches_folder(folder_name))
    }

    /// Scan a root directory and match subfolders to registered consoles.
    ///
    /// Returns a `FolderScanResult` containing matched console folders and
    /// the names of any non-hidden folders that didn't match a console.
    pub fn scan_console_folders(
        &self,
        root: &Path,
        filter: Option<&[Platform]>,
    ) -> std::io::Result<FolderScanResult> {
        let mut matches = Vec::new();
        let mut unrecognized = Vec::new();

        let mut dir_entries: Vec<std::fs::DirEntry> =
            std::fs::read_dir(root)?.flatten().collect();
        dir_entries.sort_by_key(|e| e.path());

        for entry in dir_entries {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }

            let folder_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(name) => name.to_string(),
                None => continue,
            };

            let matching_consoles = self.find_by_folder(&folder_name);
            if matching_consoles.is_empty() {
                if !folder_name.starts_with('.') {
                    unrecognized.push(folder_name);
                }
                continue;
            }

            let consoles_to_use: Vec<_> = if let Some(platforms) = filter {
                matching_consoles
                    .into_iter()
                    .filter(|c| platforms.contains(&c.metadata.platform))
                    .collect()
            } else {
                matching_consoles
            };

            for console in consoles_to_use {
                matches.push(ConsoleFolder {
                    path: path.clone(),
                    folder_name: folder_name.clone(),
                    platform: console.metadata.platform,
                });
            }
        }

        Ok(FolderScanResult {
            matches,
            unrecognized,
        })
    }
}

/// A console folder matched during a root directory scan.
#[derive(Debug, Clone)]
pub struct ConsoleFolder {
    /// Path to the folder.
    pub path: PathBuf,
    /// Name of the folder (e.g., "snes", "n3ds").
    pub folder_name: String,
    /// The platform this folder was matched to.
    pub platform: Platform,
}

/// Result of scanning a root directory for console folders.
#[derive(Debug)]
pub struct FolderScanResult {
    /// Folders that matched a registered console.
    pub matches: Vec<ConsoleFolder>,
    /// Non-hidden folder names that didn't match any console.
    pub unrecognized: Vec<String>,
}

