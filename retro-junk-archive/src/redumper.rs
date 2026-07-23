use std::fs::File;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use crate::{ArchivedFile, ToolRecord, TrackDigest, hash_file_digests};

#[derive(Debug, thiserror::Error)]
pub enum RedumperError {
    #[error("redumper raw set is missing .scram/.scrap/.sdram/.sbram in {0}")]
    MissingRawImage(String),
    #[error("redumper is unavailable: {0}")]
    Unavailable(String),
    #[error("redumper {phase} failed: {detail}")]
    CommandFailed { phase: &'static str, detail: String },
    #[error("redumper audit was cancelled")]
    Cancelled,
    #[error("could not parse redumper track hashes: {0}")]
    InvalidOutput(String),
    #[error("symbolic links are not accepted in a preservation-master audit: {0}")]
    SymbolicLink(String),
    #[error("I/O error for {path}: {source}")]
    Io {
        path: String,
        source: std::io::Error,
    },
}

#[derive(Debug, Clone)]
pub struct Redumper {
    pub path: PathBuf,
    pub version: String,
}

#[derive(Debug, Clone)]
pub struct RedumperAudit {
    pub tool: ToolRecord,
    pub tracks: Vec<TrackDigest>,
    pub log: String,
}

/// Disposable, split Redumper representation. Dropping this value removes
/// the owned scratch directory, including generated BIN/CUE files.
pub struct RedumperWorkspace {
    pub entrypoint: PathBuf,
    pub audit: RedumperAudit,
    _package: retro_junk_io::PreparedPackage,
    _guard: WorkspaceGuard,
}

impl RedumperWorkspace {
    /// Atomically retain only the emulator-oriented split output. Raw input
    /// copies and Redumper logs remain disposable workspace data.
    pub fn retain_intermediate(
        &self,
        destination: &Path,
        cancel: &AtomicBool,
    ) -> Result<Vec<ArchivedFile>, RedumperError> {
        if destination.exists() {
            return Err(RedumperError::Io {
                path: destination.display().to_string(),
                source: std::io::Error::new(
                    std::io::ErrorKind::AlreadyExists,
                    "intermediate destination already exists",
                ),
            });
        }
        let parent = destination.parent().ok_or_else(|| RedumperError::Io {
            path: destination.display().to_string(),
            source: std::io::Error::new(std::io::ErrorKind::InvalidInput, "no parent directory"),
        })?;
        std::fs::create_dir_all(parent).map_err(|source| RedumperError::Io {
            path: parent.display().to_string(),
            source,
        })?;
        let staging = parent.join(format!(".intermediate-staging-{}", uuid::Uuid::now_v7()));
        let raw = staging.join("raw");
        std::fs::create_dir_all(&raw).map_err(|source| RedumperError::Io {
            path: raw.display().to_string(),
            source,
        })?;
        let result = (|| {
            let mut archived = Vec::new();
            for source_path in sorted_files(self.entrypoint.parent().unwrap_or(Path::new(".")))? {
                if cancel.load(Ordering::Relaxed) {
                    return Err(RedumperError::Cancelled);
                }
                let extension = source_path
                    .extension()
                    .and_then(|value| value.to_str())
                    .unwrap_or_default();
                if !matches!(
                    extension.to_ascii_lowercase().as_str(),
                    "cue" | "bin" | "iso"
                ) {
                    continue;
                }
                let name = source_path.file_name().ok_or_else(|| RedumperError::Io {
                    path: source_path.display().to_string(),
                    source: std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "missing filename",
                    ),
                })?;
                let target = raw.join(name);
                std::fs::copy(&source_path, &target).map_err(|source| RedumperError::Io {
                    path: target.display().to_string(),
                    source,
                })?;
                let digests = hash_file_digests(&target, cancel).map_err(|error| {
                    RedumperError::InvalidOutput(format!(
                        "could not hash retained intermediate: {error}"
                    ))
                })?;
                archived.push(ArchivedFile {
                    path: name.to_string_lossy().into_owned(),
                    size: digests.size,
                    crc32: digests.crc32,
                    md5: digests.md5,
                    sha1: digests.sha1,
                    sha256: digests.sha256,
                });
            }
            if archived.is_empty() {
                return Err(RedumperError::InvalidOutput(
                    "no CUE/BIN or ISO files were available to retain".to_owned(),
                ));
            }
            archived.sort_by(|a, b| a.path.cmp(&b.path));
            std::fs::rename(&staging, destination).map_err(|source| RedumperError::Io {
                path: destination.display().to_string(),
                source,
            })?;
            Ok(archived)
        })();
        if result.is_err() {
            let _ = std::fs::remove_dir_all(&staging);
        }
        result
    }
}

impl Redumper {
    pub fn detect(path: &Path) -> Result<Self, RedumperError> {
        let path = if path.as_os_str().is_empty() {
            PathBuf::from("redumper")
        } else {
            path.to_path_buf()
        };
        // Never invoke redumper without a command: that begins the aggregate
        // physical-disc pipeline on systems with a configured drive.
        let output = Command::new(&path)
            .arg("--help")
            .output()
            .map_err(|error| RedumperError::Unavailable(error.to_string()))?;
        let banner = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        if !banner.to_ascii_lowercase().contains("redumper") {
            return Err(RedumperError::Unavailable(format!(
                "{} did not identify itself as redumper",
                path.display()
            )));
        }
        Ok(Self {
            path,
            version: parse_version(&banner),
        })
    }

    /// Regenerate track files from a disposable copy of a raw dump and parse
    /// the Logiqx ROM records emitted by redumper. The archive is never passed
    /// to redumper directly.
    pub fn audit(
        &self,
        raw_directory: &Path,
        workspace_root: &Path,
        cancel: &AtomicBool,
    ) -> Result<RedumperAudit, RedumperError> {
        self.audit_with_progress(raw_directory, workspace_root, cancel, |_, _| {})
    }

    pub fn audit_with_progress(
        &self,
        raw_directory: &Path,
        workspace_root: &Path,
        cancel: &AtomicBool,
        progress: impl FnMut(u64, u64),
    ) -> Result<RedumperAudit, RedumperError> {
        Ok(self
            .prepare_with_progress(raw_directory, workspace_root, cancel, progress)?
            .audit)
    }

    pub fn prepare(
        &self,
        raw_directory: &Path,
        workspace_root: &Path,
        cancel: &AtomicBool,
    ) -> Result<RedumperWorkspace, RedumperError> {
        self.prepare_with_progress(raw_directory, workspace_root, cancel, |_, _| {})
    }

    pub fn prepare_with_progress(
        &self,
        raw_directory: &Path,
        workspace_root: &Path,
        cancel: &AtomicBool,
        mut progress: impl FnMut(u64, u64),
    ) -> Result<RedumperWorkspace, RedumperError> {
        let image_name = find_image_name(raw_directory)?;
        let plan = retro_junk_io::plan_package(raw_directory)
            .map_err(|error| RedumperError::InvalidOutput(error.to_string()))?;
        let total = plan.total_bytes;
        let mut completed = 0_u64;
        let operation_workspace =
            workspace_root.join(format!("redumper-audit-{}", uuid::Uuid::now_v7()));
        let guard = WorkspaceGuard(operation_workspace.clone());
        progress(0, total);
        let package =
            retro_junk_io::stage_planned_package(&plan, &operation_workspace, cancel, |bytes| {
                completed = completed.saturating_add(bytes);
                progress(completed, total);
            })
            .map_err(|error| RedumperError::InvalidOutput(error.to_string()))?;
        let workspace = package.local_source.clone();

        let split = run_phase(&self.path, "split", &workspace, &image_name, true, cancel)?;
        let hash = run_phase(&self.path, "hash", &workspace, &image_name, false, cancel)?;
        let mut log = format!("{split}\n{hash}");
        for entry in sorted_files(&workspace)? {
            if entry.extension().and_then(|value| value.to_str()) == Some("log") {
                let text = std::fs::read_to_string(&entry).map_err(|source| RedumperError::Io {
                    path: entry.display().to_string(),
                    source,
                })?;
                log.push('\n');
                log.push_str(&text);
            }
        }
        let roms = retro_junk_dat::parse_logiqx_rom_lines(&log)
            .map_err(|error| RedumperError::InvalidOutput(error.to_string()))?;
        let unique = roms
            .into_iter()
            .filter(|rom| rom.name.to_ascii_lowercase().ends_with(".bin"))
            .map(|rom| (rom.name.clone(), rom))
            .collect::<std::collections::BTreeMap<_, _>>();
        let tracks = unique
            .into_values()
            .enumerate()
            .map(|(index, rom)| TrackDigest {
                number: u32::try_from(index + 1).unwrap_or(u32::MAX),
                size: rom.size,
                crc32: rom.crc,
                md5: rom.md5.unwrap_or_default(),
                sha1: rom.sha1.unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        if tracks.is_empty() {
            return Err(RedumperError::InvalidOutput(
                "no BIN track records were emitted".to_owned(),
            ));
        }
        let audit = RedumperAudit {
            tool: ToolRecord {
                name: "redumper".to_owned(),
                version: self.version.clone(),
                build: self.version.clone(),
            },
            tracks,
            log,
        };
        let cue = workspace.join(format!("{image_name}.cue"));
        let iso = workspace.join(format!("{image_name}.iso"));
        let entrypoint = if cue.is_file() {
            cue
        } else if iso.is_file() {
            iso
        } else {
            return Err(RedumperError::InvalidOutput(
                "split completed without producing a CUE or ISO".to_owned(),
            ));
        };
        Ok(RedumperWorkspace {
            entrypoint,
            audit,
            _package: package,
            _guard: guard,
        })
    }
}

struct WorkspaceGuard(PathBuf);

impl Drop for WorkspaceGuard {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            log::warn!(
                "could not remove Redumper audit workspace {}: {error}",
                self.0.display()
            );
        }
    }
}

fn find_image_name(directory: &Path) -> Result<String, RedumperError> {
    for path in sorted_files(directory)? {
        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();
        if matches!(extension.as_str(), "scram" | "scrap" | "sdram" | "sbram") {
            return path
                .file_stem()
                .and_then(|value| value.to_str())
                .map(str::to_owned)
                .ok_or_else(|| RedumperError::MissingRawImage(directory.display().to_string()));
        }
    }
    Err(RedumperError::MissingRawImage(
        directory.display().to_string(),
    ))
}

fn sorted_files(directory: &Path) -> Result<Vec<PathBuf>, RedumperError> {
    let mut entries = std::fs::read_dir(directory)
        .map_err(|source| RedumperError::Io {
            path: directory.display().to_string(),
            source,
        })?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| RedumperError::Io {
            path: directory.display().to_string(),
            source,
        })?;
    entries.sort_by_key(std::fs::DirEntry::path);
    Ok(entries
        .into_iter()
        .map(|entry| entry.path())
        .filter(|path| path.is_file())
        .collect())
}

fn run_phase(
    executable: &Path,
    phase: &'static str,
    workspace: &Path,
    image_name: &str,
    overwrite: bool,
    cancel: &AtomicBool,
) -> Result<String, RedumperError> {
    let stdout_path = workspace.join(format!(".retro-junk-{phase}.stdout"));
    let stderr_path = workspace.join(format!(".retro-junk-{phase}.stderr"));
    let stdout = File::create(&stdout_path).map_err(|source| RedumperError::Io {
        path: stdout_path.display().to_string(),
        source,
    })?;
    let stderr = File::create(&stderr_path).map_err(|source| RedumperError::Io {
        path: stderr_path.display().to_string(),
        source,
    })?;
    let mut command = Command::new(executable);
    command
        .current_dir(workspace)
        .arg(phase)
        .arg(format!("--image-name={image_name}"));
    if overwrite {
        command.arg("--overwrite");
    }
    let mut child = command
        .stdout(Stdio::from(stdout))
        .stderr(Stdio::from(stderr))
        .spawn()
        .map_err(|error| RedumperError::Unavailable(error.to_string()))?;
    let status = loop {
        if cancel.load(Ordering::Relaxed) {
            let _ = child.kill();
            let _ = child.wait();
            return Err(RedumperError::Cancelled);
        }
        if let Some(status) = child.try_wait().map_err(|source| RedumperError::Io {
            path: executable.display().to_string(),
            source,
        })? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let stdout = std::fs::read_to_string(&stdout_path).unwrap_or_default();
    let stderr = std::fs::read_to_string(&stderr_path).unwrap_or_default();
    let combined = format!("{stdout}\n{stderr}");
    if !status.success() {
        let detail = combined
            .lines()
            .rev()
            .take(12)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(RedumperError::CommandFailed { phase, detail });
    }
    Ok(combined)
}

fn parse_version(banner: &str) -> String {
    banner
        .lines()
        .find(|line| line.to_ascii_lowercase().contains("redumper"))
        .unwrap_or_default()
        .trim()
        .to_owned()
}
