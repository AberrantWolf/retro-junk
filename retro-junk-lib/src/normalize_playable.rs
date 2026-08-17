//! Safely converge a verified multi-disc playable onto ES-DE's `.m3u` layout.

use std::collections::{BTreeSet, HashSet};
use std::path::{Path, PathBuf};

use retro_junk_archive::{ArchiveIndexSnapshot, BuildEvidence, IndexedDump};
use retro_junk_core::disc::DiscPosition;

#[derive(Debug, Clone)]
pub struct NormalizeItem {
    pub representation_id: String,
    pub position: DiscPosition,
    pub canonical_file_name: String,
}

pub struct NormalizePlayableRequest<'a> {
    pub snapshot: &'a ArchiveIndexSnapshot,
    pub playable_root: &'a Path,
    pub archive_release_id: &'a str,
    pub playable_platform_id: &'a str,
    pub canonical_release_name: &'a str,
    pub expected_disc_count: usize,
    pub items: &'a [NormalizeItem],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedMove {
    pub from: PathBuf,
    pub to: PathBuf,
}

#[derive(Debug, Clone)]
struct EvidenceWrite {
    path: PathBuf,
    json: String,
}

#[derive(Debug, Clone)]
pub struct NormalizePlayablePlan {
    pub directory: PathBuf,
    pub playlist: PathBuf,
    pub playlist_contents: String,
    pub moves: Vec<PlannedMove>,
    directories: Vec<PathBuf>,
    evidence: Vec<EvidenceWrite>,
}

impl NormalizePlayablePlan {
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.moves.is_empty()
            && std::fs::read_to_string(&self.playlist)
                .is_ok_and(|current| current == self.playlist_contents)
    }

    pub fn commit(self) -> Result<(), NormalizePlayableError> {
        if self.is_noop() {
            return Ok(());
        }
        let mut transaction = crate::fs_txn::FsTransaction::new();
        for directory in self.directories {
            transaction.create_dir(directory);
        }
        for movement in self.moves {
            transaction.rename(movement.from, movement.to);
        }
        transaction.write_file(&self.playlist, self.playlist_contents);
        for evidence in self.evidence {
            transaction.write_file(evidence.path, evidence.json);
        }
        transaction
            .commit()
            .map(|_| ())
            .map_err(|error| NormalizePlayableError::Unsafe(error.to_string()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum NormalizePlayableError {
    #[error("not a complete, unambiguous multi-disc playable: {0}")]
    NotRepairable(String),
    #[error("playable layout conflict: {0}")]
    Unsafe(String),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("archive error: {0}")]
    Archive(String),
}

struct Located<'a> {
    dump: &'a IndexedDump,
    current: &'a BuildEvidence,
    relative: String,
    copy_id: String,
}

fn locate<'a>(
    request: &NormalizePlayableRequest<'a>,
    representation_id: &str,
) -> Option<Located<'a>> {
    let release = request.snapshot.releases.iter().find(|release| {
        release.manifest.archive_release_id.to_string() == request.archive_release_id
    })?;
    for copy in &release.physical_copies {
        for carrier in &copy.carriers {
            for dump in &carrier.dumps {
                if let Some(current) = retro_junk_archive::current_build_evidence(dump)
                    .into_iter()
                    .find(|evidence| {
                        evidence.child_representation_id.to_string() == representation_id
                    })
                {
                    return Some(Located {
                        dump,
                        current,
                        relative: crate::playable_location::release_output_relative(
                            release,
                            request.playable_root,
                            current,
                        ),
                        copy_id: copy.manifest.physical_copy_id.to_string(),
                    });
                }
            }
        }
    }
    None
}

/// Build and fully preflight a repair. No filesystem mutation occurs here.
#[allow(clippy::too_many_lines)] // one linear safety preflight; splitting would hide the all-or-nothing invariants
pub fn plan_normalize_playable(
    request: &NormalizePlayableRequest<'_>,
) -> Result<NormalizePlayablePlan, NormalizePlayableError> {
    if request.expected_disc_count < 2 || request.items.len() != request.expected_disc_count {
        return Err(NormalizePlayableError::NotRepairable(format!(
            "have {} playable discs, expected {}",
            request.items.len(),
            request.expected_disc_count
        )));
    }
    let mut position_keys = HashSet::new();
    let mut schemes = BTreeSet::new();
    for item in request.items {
        if !position_keys.insert(item.position.designator()) {
            return Err(NormalizePlayableError::NotRepairable(
                "two playable files claim the same disc position".to_owned(),
            ));
        }
        schemes.insert(matches!(item.position, DiscPosition::Alphabetic(_)));
    }
    if schemes.len() != 1 {
        return Err(NormalizePlayableError::NotRepairable(
            "numeric and alphabetic disc positions are mixed".to_owned(),
        ));
    }

    let platform_root = request
        .playable_root
        .join(retro_junk_archive::slugify(request.playable_platform_id));
    let canonical_platform = std::fs::canonicalize(&platform_root)?;
    let directory = platform_root.join(format!("{}.m3u", request.canonical_release_name));
    let playlist = directory.join(format!("{}.m3u", request.canonical_release_name));
    let mut ordered = Vec::new();
    let mut copy_id = None;
    let mut target_names = HashSet::new();
    let mut moves = Vec::new();
    let mut evidence = Vec::new();
    let mut directories = vec![directory.clone()];
    let mut stale_directories = HashSet::new();
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let backup_stamp = chrono::Utc::now().format("%Y%m%dT%H%M%S%.fZ").to_string();

    for item in request.items {
        if !target_names.insert(item.canonical_file_name.to_ascii_lowercase()) {
            return Err(NormalizePlayableError::NotRepairable(
                "canonical disc filenames collide".to_owned(),
            ));
        }
        let located = locate(request, &item.representation_id).ok_or_else(|| {
            NormalizePlayableError::NotRepairable(format!(
                "no current build evidence for {}",
                item.representation_id
            ))
        })?;
        if copy_id
            .as_ref()
            .is_some_and(|known| known != &located.copy_id)
        {
            return Err(NormalizePlayableError::NotRepairable(
                "playable discs come from different physical copies".to_owned(),
            ));
        }
        copy_id.get_or_insert(located.copy_id);
        let source = request.playable_root.join(&located.relative);
        let canonical_source = std::fs::canonicalize(&source)?;
        if !canonical_source.starts_with(&canonical_platform) || !canonical_source.is_file() {
            return Err(NormalizePlayableError::Unsafe(format!(
                "source escapes its platform root: {}",
                source.display()
            )));
        }
        let digest = retro_junk_archive::hash_file_digests(&source, &cancelled)
            .map_err(|error| NormalizePlayableError::Archive(error.to_string()))?;
        if digest.size != located.current.output_size
            || !located
                .current
                .output_sha256
                .eq_ignore_ascii_case(&digest.sha256)
        {
            return Err(NormalizePlayableError::NotRepairable(format!(
                "{} no longer matches its build evidence",
                source.display()
            )));
        }

        let stale_parent = source.parent().filter(|parent| {
            *parent != directory
                && parent
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.to_ascii_lowercase().ends_with(".m3u"))
        });
        if let Some(parent) = stale_parent {
            stale_directories.insert(parent.to_path_buf());
        }

        let target = directory.join(&item.canonical_file_name);
        let published = target.clone();
        let relocated = source != target;
        let move_target = if source == target {
            None
        } else if target.exists() {
            let target_digest = retro_junk_archive::hash_file_digests(&target, &cancelled)
                .map_err(|error| NormalizePlayableError::Archive(error.to_string()))?;
            if target_digest.size != digest.size || target_digest.sha256 != digest.sha256 {
                return Err(NormalizePlayableError::Unsafe(format!(
                    "different bytes already exist at {}",
                    target.display()
                )));
            }
            // A stale `.m3u` directory is moved wholesale after its known
            // playable members are published. Leaving this source in place
            // keeps the residual folder recoverable without trying to create
            // a backup directory that the later directory move must occupy.
            stale_parent.is_none().then(|| {
                request
                    .playable_root
                    .join(".retro-junk-backups")
                    .join(&backup_stamp)
                    .join(&located.relative)
            })
        } else {
            Some(target.clone())
        };
        if let Some(move_target) = move_target {
            if let Some(parent) = move_target.parent() {
                // The transaction creates the canonical folder. Backup
                // parents are uncommon and may be deeper, so include them as
                // explicit operations by recording a zero-cost directory op
                // through the plan's evidence parent below.
                if move_target != target && !parent.exists() {
                    directories.push(parent.to_path_buf());
                }
            }
            moves.push(PlannedMove {
                from: source.clone(),
                to: move_target,
            });
        }
        if relocated {
            let next = BuildEvidence {
                build_id: retro_junk_archive::BuildId::new(),
                performed_at: chrono::Utc::now().to_rfc3339(),
                relative_output_path: published
                    .strip_prefix(request.playable_root)
                    .unwrap_or(&published)
                    .to_string_lossy()
                    .replace('\\', "/"),
                output_sha256: digest.sha256,
                output_size: digest.size,
                ..located.current.clone()
            };
            let path = located
                .dump
                .directory
                .join("evidence")
                .join(format!("build-{}.json", next.build_id));
            let json = serde_json::to_string_pretty(&next)
                .map_err(|error| NormalizePlayableError::Archive(error.to_string()))?;
            evidence.push(EvidenceWrite { path, json });
        }
        ordered.push((item.position.sort_key(), item.canonical_file_name.clone()));
    }
    for stale in stale_directories {
        let relative = stale.strip_prefix(request.playable_root).map_err(|_| {
            NormalizePlayableError::Unsafe(format!(
                "stale playlist directory escapes playable root: {}",
                stale.display()
            ))
        })?;
        let backup = request
            .playable_root
            .join(".retro-junk-backups")
            .join(&backup_stamp)
            .join(relative);
        if backup.exists() {
            return Err(NormalizePlayableError::Unsafe(format!(
                "backup target already exists: {}",
                backup.display()
            )));
        }
        if let Some(parent) = backup.parent() {
            directories.push(parent.to_path_buf());
        }
        moves.push(PlannedMove {
            from: stale,
            to: backup,
        });
    }
    ordered.sort_by_key(|(position, _)| *position);
    let playlist_contents = ordered
        .into_iter()
        .map(|(_, name)| name)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    Ok(NormalizePlayablePlan {
        directory,
        playlist,
        playlist_contents,
        moves,
        directories,
        evidence,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicBool;

    #[test]
    #[allow(clippy::too_many_lines)] // end-to-end archive/evidence/filesystem fixture
    fn loose_first_disc_and_broken_playlist_converge_to_one_esde_entry() {
        let temp = tempfile::tempdir().unwrap();
        let archive = temp.path().join("archive");
        let playable = temp.path().join("roms");
        retro_junk_archive::initialize_archive(
            &archive,
            &retro_junk_archive::ArchiveRootManifest::new("Test"),
        )
        .unwrap();
        let master1 = temp.path().join("master-1.bin");
        let master2 = temp.path().join("master-2.bin");
        std::fs::write(&master1, b"master one").unwrap();
        std::fs::write(&master2, b"master two").unwrap();
        let spec =
            |sequence_number, join_release, physical_copy_id| retro_junk_archive::NewCarrierDump {
                platform_id: "saturn".to_owned(),
                title: "Game".to_owned(),
                region: "japan".to_owned(),
                revision: String::new(),
                variant: String::new(),
                owner_id: "default".to_owned(),
                physical_copy_label: String::new(),
                serial: String::new(),
                sequence_number,
                carrier_label: String::new(),
                carrier_kind: retro_junk_archive::CarrierKind::OpticalDisc,
                format: retro_junk_archive::RepresentationFormat::CueBin,
                catalog_binding: retro_junk_archive::CatalogBinding::default(),
                join_release,
                source_package: retro_junk_archive::SourcePackageRecord::default(),
                expected_files: Vec::new(),
                physical_copy_id,
            };
        let first = retro_junk_archive::ingest_new_carrier_dump(
            &archive,
            &master1,
            spec(1, None, None),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();
        let second = retro_junk_archive::ingest_new_carrier_dump(
            &archive,
            &master2,
            spec(
                2,
                Some(first.release.archive_release_id),
                Some(first.physical_copy.physical_copy_id),
            ),
            &AtomicBool::new(false),
            |_| {},
        )
        .unwrap();

        let system = playable.join("saturnjp");
        let set = system.join("Game (Japan).m3u");
        let stale_set = system.join("Old Game Name.m3u");
        std::fs::create_dir_all(&stale_set).unwrap();
        let disc1 = system.join("Game (Japan) (Disc 1) (1M).chd");
        let disc2 = stale_set.join("Game (Japan) (Disc 2) (2M).chd");
        std::fs::write(&disc1, b"playable one").unwrap();
        std::fs::write(&disc2, b"playable two").unwrap();
        std::fs::write(
            stale_set.join("Old Game Name.m3u"),
            "../Game (Japan).chd\nGame (Japan) (Disc 2) (2M).chd\n",
        )
        .unwrap();

        let before = retro_junk_archive::scan_archive(&archive).unwrap();
        let dumps = &before.releases[0].physical_copies[0].carriers;
        let mut representation_ids = Vec::new();
        for (carrier, (ingested, output)) in dumps.iter().zip([(&first, &disc1), (&second, &disc2)])
        {
            let dump = &carrier.dumps[0];
            let digest =
                retro_junk_archive::hash_file_digests(output, &AtomicBool::new(false)).unwrap();
            let child = retro_junk_archive::RepresentationId::new();
            representation_ids.push(child.to_string());
            retro_junk_archive::write_build_evidence(
                &ingested.dump_directory,
                &BuildEvidence {
                    schema_version: retro_junk_archive::MANIFEST_SCHEMA_VERSION,
                    build_id: retro_junk_archive::BuildId::new(),
                    parent_representation_id: ingested.dump.representation_id,
                    child_representation_id: child,
                    performed_at: "2026-01-01T00:00:00Z".to_owned(),
                    input_manifest_sha256: dump.manifest_sha256.clone(),
                    recipe_version: 1,
                    format: retro_junk_archive::RepresentationFormat::Chd,
                    relative_output_path: output
                        .strip_prefix(&playable)
                        .unwrap()
                        .to_string_lossy()
                        .replace('\\', "/"),
                    output_sha256: digest.sha256,
                    output_size: digest.size,
                    catalog_verified: true,
                    round_trip_verified: true,
                    tool: None,
                    omitted_features: Vec::new(),
                    canonical_intermediate: None,
                },
            )
            .unwrap();
        }

        let snapshot = retro_junk_archive::scan_archive(&archive).unwrap();
        let items = vec![
            NormalizeItem {
                representation_id: representation_ids[0].clone(),
                position: DiscPosition::Numeric(1),
                canonical_file_name: "Game (Japan) (Disc 1) (1M).chd".to_owned(),
            },
            NormalizeItem {
                representation_id: representation_ids[1].clone(),
                position: DiscPosition::Numeric(2),
                canonical_file_name: "Game (Japan) (Disc 2) (2M).chd".to_owned(),
            },
        ];
        let plan = plan_normalize_playable(&NormalizePlayableRequest {
            snapshot: &snapshot,
            playable_root: &playable,
            archive_release_id: &first.release.archive_release_id.to_string(),
            playable_platform_id: "saturnjp",
            canonical_release_name: "Game (Japan)",
            expected_disc_count: 2,
            items: &items,
        })
        .unwrap();
        assert_eq!(plan.moves.len(), 3, "two discs and the stale set directory");
        plan.commit().unwrap();

        assert!(!disc1.exists());
        assert!(!stale_set.exists());
        assert!(playable.join(".retro-junk-backups").is_dir());
        assert!(set.join("Game (Japan) (Disc 1) (1M).chd").is_file());
        assert_eq!(
            std::fs::read_to_string(set.join("Game (Japan).m3u")).unwrap(),
            "Game (Japan) (Disc 1) (1M).chd\nGame (Japan) (Disc 2) (2M).chd\n"
        );
        let entries =
            crate::scanner::scan_game_entries(&system, &crate::scanner::extension_set(&["chd"]))
                .unwrap();
        assert_eq!(entries.len(), 1, "{entries:?}");
        assert_eq!(entries[0].display_name(), "Game (Japan).m3u");
        assert_eq!(entries[0].all_files().len(), 2);
    }
}
