//! Analysis context for ROM analysis.

use std::path::Path;

use retro_junk_core::RomAnalyzer;

/// Metadata about a registered console.
#[derive(Debug, Clone)]
pub struct Console {
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

    /// Get a console by short name.
    pub fn get_by_short_name(&self, short_name: &str) -> Option<&RegisteredConsole> {
        let short_lower = short_name.to_lowercase();
        self.consoles
            .iter()
            .find(|c| c.metadata.short_name.to_lowercase() == short_lower)
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

    /// Scan a root directory and return folders that match registered consoles.
    /// Returns pairs of (folder_path, matching_console).
    pub fn scan_root(
        &self,
        root: &Path,
        filter_consoles: Option<&[String]>,
    ) -> std::io::Result<Vec<(&Path, &RegisteredConsole)>> {
        // This would need to be implemented with actual directory scanning
        // For now, return empty - the CLI will handle the scanning
        let _ = (root, filter_consoles);
        Ok(Vec::new())
    }
}
