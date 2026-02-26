use std::fs;
use std::io::{self, Seek, Write};
use std::path::{Path, PathBuf};

use retro_junk_core::{AnalysisOptions, DatSource, RomAnalyzer};
use retro_junk_dat::cache;
use retro_junk_dat::error::DatError;
use retro_junk_dat::matcher::DatIndex;

use crate::hasher::{self, PaddingSpec};

/// CD pregap size: 2 seconds × 75 sectors/sec × 2352 bytes/sector = 352,800 bytes.
const CD_PREGAP_SIZE: u64 = 352_800;

/// How a repair modifies the file.
#[derive(Debug, Clone)]
pub enum RepairMethod {
    /// Append fill bytes to the end of the file.
    AppendPadding { fill_byte: u8, bytes_added: u64 },
    /// Prepend fill bytes to the beginning of the file.
    PrependPadding { fill_byte: u8, bytes_added: u64 },
}

impl RepairMethod {
    /// Human-readable description of the repair.
    pub fn description(&self) -> String {
        match self {
            RepairMethod::AppendPadding {
                fill_byte,
                bytes_added,
            } => {
                format!(
                    "append {} of 0x{:02X}",
                    format_bytes(*bytes_added),
                    fill_byte
                )
            }
            RepairMethod::PrependPadding {
                fill_byte,
                bytes_added,
            } => {
                format!(
                    "prepend {} of 0x{:02X}",
                    format_bytes(*bytes_added),
                    fill_byte
                )
            }
        }
    }
}

/// A planned repair for a single file.
#[derive(Debug, Clone)]
pub struct RepairAction {
    /// Path to the file to repair.
    pub file_path: PathBuf,
    /// Canonical game name from the DAT.
    pub game_name: String,
    /// How the file will be modified.
    pub method: RepairMethod,
    /// Padding spec used to verify the repair hash.
    pub padding: PaddingSpec,
}

/// Result of planning repairs for a console folder.
#[derive(Debug)]
pub struct RepairPlan {
    /// Files that already match the DAT (no repair needed).
    pub already_correct: Vec<PathBuf>,
    /// Files that can be repaired.
    pub repairable: Vec<RepairAction>,
    /// Files that didn't match any repair strategy.
    pub no_match: Vec<PathBuf>,
    /// Files that encountered errors during planning.
    pub errors: Vec<(PathBuf, String)>,
}

/// Options controlling repair behavior.
#[derive(Debug, Clone)]
pub struct RepairOptions {
    /// Custom DAT directory (instead of cache).
    pub dat_dir: Option<PathBuf>,
    /// Maximum number of ROMs to process.
    pub limit: Option<usize>,
    /// Whether to create .bak backup files before modifying.
    pub create_backup: bool,
}

impl Default for RepairOptions {
    fn default() -> Self {
        Self {
            dat_dir: None,
            limit: None,
            create_backup: true,
        }
    }
}

/// Progress information for callbacks.
#[derive(Debug, Clone)]
pub enum RepairProgress {
    /// Scanning the folder for ROM files.
    Scanning { file_count: usize },
    /// Checking a file against the DAT as-is.
    Checking {
        file_name: String,
        file_index: usize,
        total: usize,
    },
    /// Trying repair strategies for a file.
    TryingRepair {
        file_name: String,
        strategy_desc: String,
    },
    /// Done planning.
    Done,
}

/// Summary of an executed repair operation.
#[derive(Debug, Clone, Default)]
pub struct RepairSummary {
    pub repaired: usize,
    pub already_correct: usize,
    pub no_match: usize,
    pub errors: Vec<String>,
    pub backups_created: usize,
}

/// A candidate repair strategy to try.
struct Strategy {
    padding: PaddingSpec,
    method_fn: fn(u8, u64) -> RepairMethod,
    description: String,
}

/// Build repair strategies for a file that doesn't match the DAT.
///
/// Given the current data size and the analyzer, produces an ordered list
/// of strategies to try.
fn build_strategies(
    data_size: u64,
    expected_data_size: Option<u64>,
    dat_source: DatSource,
) -> Vec<Strategy> {
    let mut strategies = Vec::new();

    if let Some(expected) = expected_data_size {
        if expected > data_size {
            let diff = expected - data_size;
            // Try append 0x00
            strategies.push(Strategy {
                padding: PaddingSpec {
                    prepend_size: 0,
                    append_size: diff,
                    fill_byte: 0x00,
                },
                method_fn: RepairMethod::append,
                description: format!("append {} of 0x00 to expected size", format_bytes(diff)),
            });
            // Try append 0xFF
            strategies.push(Strategy {
                padding: PaddingSpec {
                    prepend_size: 0,
                    append_size: diff,
                    fill_byte: 0xFF,
                },
                method_fn: RepairMethod::append,
                description: format!("append {} of 0xFF to expected size", format_bytes(diff)),
            });
        }
    }

    // Redump disc images: try prepending CD pregap
    if dat_source == DatSource::Redump {
        strategies.push(Strategy {
            padding: PaddingSpec {
                prepend_size: CD_PREGAP_SIZE,
                append_size: 0,
                fill_byte: 0x00,
            },
            method_fn: RepairMethod::prepend,
            description: format!("prepend {} CD pregap of 0x00", format_bytes(CD_PREGAP_SIZE)),
        });
    }

    // No-Intro cartridge ROMs: if no expected size, try padding to next power of 2
    if expected_data_size.is_none()
        && dat_source == DatSource::NoIntro
        && !is_power_of_two(data_size)
    {
        let next_pow2 = data_size.next_power_of_two();
        let diff = next_pow2 - data_size;
        strategies.push(Strategy {
            padding: PaddingSpec {
                prepend_size: 0,
                append_size: diff,
                fill_byte: 0x00,
            },
            method_fn: RepairMethod::append,
            description: format!(
                "append {} of 0x00 to next power-of-2 ({})",
                format_bytes(diff),
                format_bytes(next_pow2)
            ),
        });
        strategies.push(Strategy {
            padding: PaddingSpec {
                prepend_size: 0,
                append_size: diff,
                fill_byte: 0xFF,
            },
            method_fn: RepairMethod::append,
            description: format!(
                "append {} of 0xFF to next power-of-2 ({})",
                format_bytes(diff),
                format_bytes(next_pow2)
            ),
        });
    }

    strategies
}

impl RepairMethod {
    fn append(fill_byte: u8, bytes_added: u64) -> RepairMethod {
        RepairMethod::AppendPadding {
            fill_byte,
            bytes_added,
        }
    }

    fn prepend(fill_byte: u8, bytes_added: u64) -> RepairMethod {
        RepairMethod::PrependPadding {
            fill_byte,
            bytes_added,
        }
    }
}

/// Plan repairs for a single console folder.
pub fn plan_repairs(
    folder: &Path,
    analyzer: &dyn RomAnalyzer,
    options: &RepairOptions,
    progress: &dyn Fn(RepairProgress),
) -> Result<RepairPlan, DatError> {
    let dat_names = analyzer.dat_names();
    if dat_names.is_empty() {
        return Err(DatError::cache(format!(
            "No DAT support for platform '{}'",
            analyzer.platform_name()
        )));
    }

    let dat_source = analyzer.dat_source();
    let download_ids = analyzer.dat_download_ids();
    let dats = cache::load_dats(
        analyzer.short_name(),
        dat_names,
        download_ids,
        options.dat_dir.as_deref(),
        dat_source,
    )?;
    let index = DatIndex::from_dats(dats);

    // Collect ROM files
    let extensions = crate::scanner::extension_set(analyzer.file_extensions());
    let game_entries = crate::scanner::scan_game_entries(folder, &extensions)
        .map_err(|e| DatError::cache(format!("Error scanning {}: {}", folder.display(), e)))?;

    let mut files: Vec<PathBuf> = game_entries
        .iter()
        .flat_map(|entry| entry.all_files())
        .cloned()
        .collect();
    if let Some(max) = options.limit {
        files.truncate(max);
    }

    progress(RepairProgress::Scanning {
        file_count: files.len(),
    });

    let mut already_correct = Vec::new();
    let mut repairable = Vec::new();
    let mut no_match = Vec::new();
    let mut errors = Vec::new();
    let analysis_options = AnalysisOptions::new().quick(true);

    for (i, file_path) in files.iter().enumerate() {
        let file_name = file_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("?")
            .to_string();

        progress(RepairProgress::Checking {
            file_name: file_name.clone(),
            file_index: i,
            total: files.len(),
        });

        // Step 1: Hash file as-is and check against DAT
        let as_is_result = match hash_and_match(file_path, analyzer, &index) {
            Ok(r) => r,
            Err(e) => {
                errors.push((file_path.clone(), e.to_string()));
                continue;
            }
        };

        // Step 2: Analyze file to get expected_size
        let expected_data_size = get_expected_data_size(file_path, analyzer, &analysis_options);

        // Step 3: Compute current data_size (file_size - header)
        let data_size = match get_data_size(file_path, analyzer) {
            Ok(s) => s,
            Err(e) => {
                errors.push((file_path.clone(), e.to_string()));
                continue;
            }
        };

        // Check if file is trimmed (smaller than header-declared size)
        let is_trimmed = matches!(expected_data_size, Some(expected) if expected > data_size);

        // If the as-is hash matches and the file is NOT trimmed, it's correct
        if as_is_result.is_some() && !is_trimmed {
            already_correct.push(file_path.clone());
            continue;
        }

        // Step 4: Build and try strategies
        // Even if the as-is hash matched, a trimmed file may have a better
        // (full-size) match available in the DAT.
        let strategies = build_strategies(data_size, expected_data_size, dat_source);

        let mut found = false;
        for strategy in &strategies {
            let target_size =
                strategy.padding.prepend_size + data_size + strategy.padding.append_size;

            // Pre-filter: skip if no DAT entries at the target size
            if index.candidates_by_size(target_size).is_none() {
                continue;
            }

            progress(RepairProgress::TryingRepair {
                file_name: file_name.clone(),
                strategy_desc: strategy.description.clone(),
            });

            let mut file = match fs::File::open(file_path) {
                Ok(f) => f,
                Err(e) => {
                    errors.push((file_path.clone(), e.to_string()));
                    found = true; // Don't add to no_match
                    break;
                }
            };

            match hasher::compute_crc32_sha1_with_padding(&mut file, analyzer, &strategy.padding) {
                Ok(hashes) => {
                    if let Some(result) = index.match_by_hash(hashes.data_size, &hashes) {
                        let game = &index.games[result.game_index];
                        let method = (strategy.method_fn)(
                            strategy.padding.fill_byte,
                            if strategy.padding.prepend_size > 0 {
                                strategy.padding.prepend_size
                            } else {
                                strategy.padding.append_size
                            },
                        );
                        repairable.push(RepairAction {
                            file_path: file_path.clone(),
                            game_name: game.name.clone(),
                            method,
                            padding: strategy.padding.clone(),
                        });
                        found = true;
                        break;
                    }
                }
                Err(e) => {
                    errors.push((file_path.clone(), e.to_string()));
                    found = true;
                    break;
                }
            }
        }

        if !found {
            if as_is_result.is_some() {
                // File matches DAT as-is but is trimmed and no full-size
                // entry exists — the trimmed version is the best we have
                already_correct.push(file_path.clone());
            } else {
                no_match.push(file_path.clone());
            }
        }
    }

    progress(RepairProgress::Done);

    Ok(RepairPlan {
        already_correct,
        repairable,
        no_match,
        errors,
    })
}

/// Hash a file as-is and try to match against the DAT index.
fn hash_and_match(
    file_path: &Path,
    analyzer: &dyn RomAnalyzer,
    index: &DatIndex,
) -> Result<Option<String>, DatError> {
    let mut file = fs::File::open(file_path)?;
    let hashes = hasher::compute_crc32_sha1(&mut file, analyzer)?;

    if let Some(result) = index.match_by_hash(hashes.data_size, &hashes) {
        let game = &index.games[result.game_index];
        return Ok(Some(game.name.clone()));
    }
    Ok(None)
}

/// Get the data size of a file (file_size - header_size).
fn get_data_size(file_path: &Path, analyzer: &dyn RomAnalyzer) -> Result<u64, DatError> {
    let mut file = fs::File::open(file_path)?;
    let file_size = file.seek(io::SeekFrom::End(0))?;
    let skip = analyzer
        .dat_header_size(&mut file, file_size)
        .map_err(|e| DatError::cache(e.to_string()))?;
    Ok(file_size - skip)
}

/// Try to get expected data size from the analyzer (expected_size - header_size).
fn get_expected_data_size(
    file_path: &Path,
    analyzer: &dyn RomAnalyzer,
    analysis_options: &AnalysisOptions,
) -> Option<u64> {
    let file_options = AnalysisOptions {
        file_path: Some(file_path.to_path_buf()),
        ..analysis_options.clone()
    };
    let mut file = fs::File::open(file_path).ok()?;
    let file_size = file.seek(io::SeekFrom::End(0)).ok()?;
    let skip = analyzer.dat_header_size(&mut file, file_size).ok()?;
    let info = analyzer.analyze(&mut file, &file_options).ok()?;
    let expected = info.expected_size?;
    Some(expected.saturating_sub(skip))
}

/// Execute a repair plan, modifying files on disk.
pub fn execute_repairs(plan: &RepairPlan, create_backup: bool) -> RepairSummary {
    let mut summary = RepairSummary {
        already_correct: plan.already_correct.len(),
        no_match: plan.no_match.len(),
        ..Default::default()
    };

    for action in &plan.repairable {
        // Create backup if requested
        if create_backup {
            let bak_path = action.file_path.with_extension(format!(
                "{}.bak",
                action
                    .file_path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
            ));
            if !bak_path.exists() {
                match fs::copy(&action.file_path, &bak_path) {
                    Ok(_) => summary.backups_created += 1,
                    Err(e) => {
                        summary.errors.push(format!(
                            "Failed to create backup for {}: {}",
                            action.file_path.display(),
                            e,
                        ));
                        continue;
                    }
                }
            }
        }

        match &action.method {
            RepairMethod::AppendPadding {
                fill_byte,
                bytes_added,
            } => match append_to_file(&action.file_path, *fill_byte, *bytes_added) {
                Ok(()) => summary.repaired += 1,
                Err(e) => {
                    summary.errors.push(format!(
                        "Failed to repair {}: {}",
                        action.file_path.display(),
                        e,
                    ));
                }
            },
            RepairMethod::PrependPadding {
                fill_byte,
                bytes_added,
            } => match prepend_to_file(&action.file_path, *fill_byte, *bytes_added) {
                Ok(()) => summary.repaired += 1,
                Err(e) => {
                    summary.errors.push(format!(
                        "Failed to repair {}: {}",
                        action.file_path.display(),
                        e,
                    ));
                }
            },
        }
    }

    summary
}

/// Append fill bytes to the end of a file.
fn append_to_file(path: &Path, fill_byte: u8, count: u64) -> io::Result<()> {
    let mut file = fs::OpenOptions::new().append(true).open(path)?;
    let buf = vec![fill_byte; 64 * 1024];
    let mut remaining = count;
    while remaining > 0 {
        let n = std::cmp::min(remaining, buf.len() as u64) as usize;
        file.write_all(&buf[..n])?;
        remaining -= n as u64;
    }
    file.flush()?;
    Ok(())
}

/// Prepend fill bytes to the beginning of a file using a temp file.
fn prepend_to_file(path: &Path, fill_byte: u8, count: u64) -> io::Result<()> {
    let tmp_path = path.with_extension("repair_tmp");

    // Write fill bytes + original content to temp file
    let mut tmp = fs::File::create(&tmp_path)?;
    let buf = vec![fill_byte; 64 * 1024];
    let mut remaining = count;
    while remaining > 0 {
        let n = std::cmp::min(remaining, buf.len() as u64) as usize;
        tmp.write_all(&buf[..n])?;
        remaining -= n as u64;
    }

    let mut original = fs::File::open(path)?;
    io::copy(&mut original, &mut tmp)?;
    tmp.flush()?;
    drop(tmp);
    drop(original);

    // Rename temp over original
    fs::rename(&tmp_path, path)?;
    Ok(())
}

fn is_power_of_two(n: u64) -> bool {
    n > 0 && (n & (n - 1)) == 0
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 && bytes % (1024 * 1024) == 0 {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 && bytes % 1024 == 0 {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} bytes", bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_power_of_two() {
        assert!(is_power_of_two(1));
        assert!(is_power_of_two(2));
        assert!(is_power_of_two(1024));
        assert!(is_power_of_two(1048576));
        assert!(!is_power_of_two(0));
        assert!(!is_power_of_two(3));
        assert!(!is_power_of_two(1000));
    }

    #[test]
    fn test_build_strategies_with_expected_size() {
        // File is 2 MB, expected 4 MB — should get append 0x00 and 0xFF strategies
        let strategies =
            build_strategies(2 * 1024 * 1024, Some(4 * 1024 * 1024), DatSource::NoIntro);
        assert_eq!(strategies.len(), 2);
        assert_eq!(strategies[0].padding.append_size, 2 * 1024 * 1024);
        assert_eq!(strategies[0].padding.fill_byte, 0x00);
        assert_eq!(strategies[1].padding.fill_byte, 0xFF);
    }

    #[test]
    fn test_build_strategies_redump_pregap() {
        // Redump disc image with no expected size
        let strategies = build_strategies(650_000_000, None, DatSource::Redump);
        assert_eq!(strategies.len(), 1);
        assert_eq!(strategies[0].padding.prepend_size, CD_PREGAP_SIZE);
        assert_eq!(strategies[0].padding.fill_byte, 0x00);
    }

    #[test]
    fn test_build_strategies_no_expected_non_pow2() {
        // 3 MB NoIntro ROM with no expected size — should try padding to 4 MB
        let strategies = build_strategies(3 * 1024 * 1024, None, DatSource::NoIntro);
        assert_eq!(strategies.len(), 2);
        assert_eq!(strategies[0].padding.append_size, 1024 * 1024);
        assert_eq!(strategies[0].padding.fill_byte, 0x00);
        assert_eq!(strategies[1].padding.fill_byte, 0xFF);
    }

    #[test]
    fn test_build_strategies_already_pow2_no_expected() {
        // 4 MB NoIntro ROM with no expected size — already power of 2, no strategies
        let strategies = build_strategies(4 * 1024 * 1024, None, DatSource::NoIntro);
        assert!(strategies.is_empty());
    }

    #[test]
    fn test_build_strategies_expected_plus_redump() {
        // Redump with expected size: should get append strategies + pregap strategy
        let strategies = build_strategies(600_000_000, Some(650_000_000), DatSource::Redump);
        assert_eq!(strategies.len(), 3); // append 0x00, append 0xFF, prepend pregap
    }

    #[test]
    fn test_repair_method_description() {
        let m = RepairMethod::AppendPadding {
            fill_byte: 0x00,
            bytes_added: 1048576,
        };
        assert_eq!(m.description(), "append 1 MB of 0x00");

        let m = RepairMethod::PrependPadding {
            fill_byte: 0x00,
            bytes_added: 352800,
        };
        assert_eq!(m.description(), "prepend 352800 bytes of 0x00");
    }

    #[test]
    fn test_backup_extension() {
        // Verify the backup path construction
        let path = PathBuf::from("/roms/snes/game.sfc");
        let bak_path = path.with_extension(format!(
            "{}.bak",
            path.extension().and_then(|e| e.to_str()).unwrap_or("")
        ));
        assert_eq!(bak_path, PathBuf::from("/roms/snes/game.sfc.bak"));
    }
}
