//! Serialized `SQLite` worker and revision-aware UI projection controller.
#![allow(dead_code)] // APIs are consumed incrementally as producers cut over.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread;

use retro_junk_db::{
    EntryAnalysisUpdate, LibraryChangeSet, LibraryConsoleDescriptor, LibraryConsoleId,
    LibraryConsoleSummary, LibraryEntryDetail, LibraryEntryId, LibraryEntryListPage,
    LibraryEntryListQuery, LibraryRootId, ScannedLibraryEntry,
};

pub type UiSessionGeneration = u64;
pub type StoreRequestId = u64;

#[derive(Debug, Clone)]
pub struct CompletedConsoleScan {
    pub root_path: String,
    pub platform: String,
    pub folder_name: String,
    pub folder_path: String,
    pub fingerprint_hash: String,
    pub entries: Vec<ScannedLibraryEntry>,
}

#[derive(Debug, Clone)]
pub struct StoreEnvelope<T> {
    pub session_generation: UiSessionGeneration,
    pub request_id: StoreRequestId,
    pub payload: T,
}

#[derive(Debug, Clone)]
pub enum LibraryStoreRequest {
    OpenRoot(String),
    EnsureConsole(LibraryConsoleDescriptor),
    CommitConsoleScan(CompletedConsoleScan),
    ConsoleSummaries(LibraryRootId),
    EntryList(LibraryEntryListQuery),
    EntryDetail(LibraryEntryId),
    EntryDetails(Vec<LibraryEntryId>),
    ConsoleEntryDetails(LibraryConsoleId),
    SetRegionOverride {
        entry_id: LibraryEntryId,
        value: Option<String>,
    },
    /// Every tagging request carries the collection root, because the
    /// decision has to be recorded beside the files as well as in the row:
    /// this database is rebuilt from DATs, and these are exactly the files no
    /// DAT describes.
    SetTag {
        entry_id: LibraryEntryId,
        value: Option<String>,
        collection_root: Option<std::path::PathBuf>,
    },
    CreateHomebrewAndTag {
        entry_id: LibraryEntryId,
        name: String,
        platform_id: String,
        region: String,
        collection_root: Option<std::path::PathBuf>,
    },
    CreateModdedAndTag {
        entry_id: LibraryEntryId,
        work_id: String,
        platform_id: String,
        region: String,
        disc_number: Option<u32>,
        hashes: Option<retro_junk_db::MediaHashes>,
        collection_root: Option<std::path::PathBuf>,
    },
    ApplyAnalysis {
        entry_id: LibraryEntryId,
        expected_source_revision: u64,
        update: EntryAnalysisUpdate,
    },
    ApplyAnalysisBatch(Vec<retro_junk_db::EntryAnalysisCommand>),
    ApplyHashUpdate {
        entry_id: LibraryEntryId,
        expected_source_revision: u64,
        update: retro_junk_db::EntryHashUpdate,
    },
    ApplyFilesystemTransition {
        console_id: LibraryConsoleId,
        entry_id: LibraryEntryId,
        expected_source_revision: u64,
        entry: ScannedLibraryEntry,
    },
    MarkConsoleStale(LibraryConsoleId),
    DeleteConsoleIfEmpty(LibraryConsoleId),
    DeleteConsole(LibraryConsoleId),
    DeleteRoot(LibraryRootId),
    DeleteRootPath(String),
    ClearCache,
    Shutdown,
}

#[derive(Debug, Clone)]
pub enum LibraryStoreValue {
    RootOpened {
        root_id: LibraryRootId,
        summaries: Vec<LibraryConsoleSummary>,
    },
    ConsoleEnsured {
        folder_name: String,
        console_id: LibraryConsoleId,
    },
    ConsoleScanCommitted {
        folder_name: String,
        console_id: LibraryConsoleId,
        entry_count: u64,
        changes: LibraryChangeSet,
    },
    ConsoleSummaries(Vec<LibraryConsoleSummary>),
    EntryList(LibraryEntryListPage),
    EntryDetail(Option<LibraryEntryDetail>),
    EntryDetails(Vec<LibraryEntryDetail>),
    ConsoleDetails {
        entries: Vec<LibraryEntryDetail>,
        archived_releases: Vec<retro_junk_db::ArchivedLibraryListItem>,
    },
    ChangeSet(LibraryChangeSet),
    ShutdownComplete,
}

pub type LibraryStoreReply = StoreEnvelope<Result<LibraryStoreValue, String>>;

pub struct LibraryStore {
    write_tx: mpsc::SyncSender<StoreEnvelope<LibraryStoreRequest>>,
    read_tx: mpsc::SyncSender<StoreEnvelope<LibraryStoreRequest>>,
    reply_rx: mpsc::Receiver<LibraryStoreReply>,
    threads: Vec<thread::JoinHandle<()>>,
}

impl LibraryStore {
    pub fn start(path: PathBuf) -> Result<Self, String> {
        // Bound queued database work so rapid filtering/navigation cannot grow
        // memory without limit behind a long reconciliation transaction.
        let (write_tx, write_rx) = mpsc::sync_channel(256);
        let (read_tx, read_rx) = mpsc::sync_channel(256);
        let (reply_tx, reply_rx) = mpsc::channel();
        let (writer_ready_tx, writer_ready_rx) = mpsc::channel();
        let write_path = path.clone();
        let write_replies = reply_tx.clone();
        let writer = thread::Builder::new()
            .name("library-store-writer".into())
            .spawn(move || {
                worker_main(write_path, write_rx, write_replies, Some(writer_ready_tx));
            })
            .map_err(|e| e.to_string())?;
        let reader = thread::Builder::new()
            .name("library-store-reader".into())
            .spawn(move || {
                // A fresh database must finish schema initialization before a
                // second connection attempts the same migration.
                let _ = writer_ready_rx.recv();
                worker_main(path, read_rx, reply_tx, None);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            write_tx,
            read_tx,
            reply_rx,
            threads: vec![writer, reader],
        })
    }

    pub fn submit(
        &self,
        envelope: StoreEnvelope<LibraryStoreRequest>,
    ) -> Result<(), mpsc::SendError<StoreEnvelope<LibraryStoreRequest>>> {
        if is_read_request(&envelope.payload) {
            self.read_tx.send(envelope)
        } else {
            self.write_tx.send(envelope)
        }
    }

    /// Queue many requests without ever blocking the caller.
    ///
    /// `submit` blocks when the bounded queue is full. That is the right
    /// backpressure for worker threads, but the UI thread must never wait on
    /// it — a bulk action submits one request per selected row and can outrun
    /// the queue, freezing the window mid-frame. The envelopes are handed to
    /// a short-lived feeder thread that performs the same routing and
    /// (blocking) sends; ids, replies, and FIFO order within the batch are
    /// exactly as if each had been submitted directly.
    pub fn submit_batch(&self, envelopes: Vec<StoreEnvelope<LibraryStoreRequest>>) {
        let write_tx = self.write_tx.clone();
        let read_tx = self.read_tx.clone();
        let _ = thread::Builder::new()
            .name("library-store-feeder".into())
            .spawn(move || {
                for envelope in envelopes {
                    let sent = if is_read_request(&envelope.payload) {
                        read_tx.send(envelope)
                    } else {
                        write_tx.send(envelope)
                    };
                    if sent.is_err() {
                        // The store shut down; the rest have nowhere to go.
                        break;
                    }
                }
            });
    }

    pub fn try_recv(&self) -> Result<LibraryStoreReply, mpsc::TryRecvError> {
        self.reply_rx.try_recv()
    }

    pub fn recv_timeout(
        &self,
        timeout: std::time::Duration,
    ) -> Result<LibraryStoreReply, mpsc::RecvTimeoutError> {
        self.reply_rx.recv_timeout(timeout)
    }

    /// FIFO channels guarantee that all commands submitted before shutdown
    /// finish before the worker acknowledges shutdown.
    pub fn shutdown_and_join(&mut self, generation: UiSessionGeneration) {
        if self.threads.is_empty() {
            return;
        }
        let _ = self.write_tx.send(StoreEnvelope {
            session_generation: generation,
            request_id: u64::MAX,
            payload: LibraryStoreRequest::Shutdown,
        });
        let _ = self.read_tx.send(StoreEnvelope {
            session_generation: generation,
            request_id: u64::MAX - 1,
            payload: LibraryStoreRequest::Shutdown,
        });
        for handle in self.threads.drain(..) {
            let _ = handle.join();
        }
    }
}

fn is_read_request(request: &LibraryStoreRequest) -> bool {
    matches!(
        request,
        LibraryStoreRequest::ConsoleSummaries(_)
            | LibraryStoreRequest::EntryList(_)
            | LibraryStoreRequest::EntryDetail(_)
            | LibraryStoreRequest::EntryDetails(_)
            | LibraryStoreRequest::ConsoleEntryDetails(_)
    )
}

impl Drop for LibraryStore {
    fn drop(&mut self) {
        self.shutdown_and_join(0);
    }
}

fn worker_main(
    path: PathBuf,
    requests: mpsc::Receiver<StoreEnvelope<LibraryStoreRequest>>,
    replies: mpsc::Sender<LibraryStoreReply>,
    ready: Option<mpsc::Sender<()>>,
) {
    let connection = retro_junk_db::open_database(&path);
    if let Some(ready) = ready {
        let _ = ready.send(());
    }
    let mut conn = match connection {
        Ok(conn) => conn,
        Err(error) => {
            log::error!("library store failed to open: {error}");
            return;
        }
    };
    while let Ok(request) = requests.recv() {
        let stop = matches!(request.payload, LibraryStoreRequest::Shutdown);
        let payload = execute(&mut conn, request.payload).map_err(|e| e.to_string());
        let _ = replies.send(StoreEnvelope {
            session_generation: request.session_generation,
            request_id: request.request_id,
            payload,
        });
        if stop {
            break;
        }
    }
}

fn execute(
    conn: &mut retro_junk_db::Connection,
    request: LibraryStoreRequest,
) -> Result<LibraryStoreValue, retro_junk_db::LibraryError> {
    use LibraryStoreRequest as R;
    Ok(match request {
        R::OpenRoot(path) => {
            let root_id = retro_junk_db::upsert_library_root(conn, &path)?;
            let summaries = retro_junk_db::list_console_summaries(conn, root_id)?;
            LibraryStoreValue::RootOpened { root_id, summaries }
        }
        R::EnsureConsole(descriptor) => {
            let folder_name = descriptor.folder_name.clone();
            let console_id = retro_junk_db::ensure_library_console(conn, &descriptor)?;
            LibraryStoreValue::ConsoleEnsured {
                folder_name,
                console_id,
            }
        }
        R::CommitConsoleScan(snapshot) => {
            let root_id = retro_junk_db::upsert_library_root(conn, &snapshot.root_path)?;
            let console_id = retro_junk_db::ensure_library_console(
                conn,
                &LibraryConsoleDescriptor {
                    root_id,
                    platform: snapshot.platform.clone(),
                    folder_name: snapshot.folder_name.clone(),
                    folder_path: snapshot.folder_path.clone(),
                },
            )?;
            let token = retro_junk_db::begin_console_scan(conn, console_id)?;
            let changes = retro_junk_db::reconcile_console_scan(
                conn,
                token,
                &snapshot.fingerprint_hash,
                &snapshot.entries,
            )?;
            LibraryStoreValue::ConsoleScanCommitted {
                folder_name: snapshot.folder_name,
                console_id,
                entry_count: snapshot.entries.len() as u64,
                changes,
            }
        }
        R::ConsoleSummaries(root) => {
            LibraryStoreValue::ConsoleSummaries(retro_junk_db::list_console_summaries(conn, root)?)
        }
        R::EntryList(query) => {
            LibraryStoreValue::EntryList(retro_junk_db::query_entry_list(conn, &query)?)
        }
        R::EntryDetail(id) => {
            LibraryStoreValue::EntryDetail(retro_junk_db::load_entry_detail(conn, id)?)
        }
        R::EntryDetails(ids) => {
            LibraryStoreValue::EntryDetails(retro_junk_db::load_entry_details(conn, &ids)?)
        }
        R::ConsoleEntryDetails(console_id) => LibraryStoreValue::ConsoleDetails {
            entries: retro_junk_db::load_entry_details_for_console(conn, console_id)?,
            archived_releases: retro_junk_db::load_archived_library_releases_for_console(
                conn, console_id,
            )?,
        },
        R::SetRegionOverride { entry_id, value } => LibraryStoreValue::ChangeSet(
            retro_junk_db::set_entry_region_override(conn, entry_id, value.as_deref())?,
        ),
        R::SetTag {
            entry_id,
            value,
            collection_root,
        } => LibraryStoreValue::ChangeSet(retro_junk_db::set_entry_tag(
            conn,
            entry_id,
            value.as_deref(),
            collection_root.as_deref(),
        )?),
        R::CreateHomebrewAndTag {
            entry_id,
            name,
            platform_id,
            region,
            collection_root,
        } => LibraryStoreValue::ChangeSet(retro_junk_db::create_homebrew_and_tag_entry(
            conn,
            entry_id,
            &name,
            &platform_id,
            &region,
            collection_root.as_deref(),
        )?),
        R::CreateModdedAndTag {
            entry_id,
            work_id,
            platform_id,
            region,
            disc_number,
            hashes,
            collection_root,
        } => LibraryStoreValue::ChangeSet(retro_junk_db::create_modded_and_tag_entry(
            conn,
            entry_id,
            &retro_junk_db::ModdedEntry {
                work_id: &work_id,
                platform_id: &platform_id,
                region: &region,
                disc_number,
                hashes: hashes.as_ref(),
                collection_root: collection_root.as_deref(),
            },
        )?),
        R::ApplyAnalysis {
            entry_id,
            expected_source_revision,
            update,
        } => LibraryStoreValue::ChangeSet(retro_junk_db::apply_entry_analysis(
            conn,
            entry_id,
            expected_source_revision,
            &update,
        )?),
        R::ApplyAnalysisBatch(commands) => LibraryStoreValue::ChangeSet(
            retro_junk_db::apply_entry_analysis_batch(conn, &commands)?,
        ),
        R::ApplyHashUpdate {
            entry_id,
            expected_source_revision,
            update,
        } => LibraryStoreValue::ChangeSet(retro_junk_db::apply_entry_hash_update(
            conn,
            entry_id,
            expected_source_revision,
            &update,
        )?),
        R::ApplyFilesystemTransition {
            console_id,
            entry_id,
            expected_source_revision,
            entry,
        } => {
            match retro_junk_db::apply_filesystem_transition(
                conn,
                entry_id,
                expected_source_revision,
                &entry,
            ) {
                Ok(changes) => LibraryStoreValue::ChangeSet(changes),
                Err(error) => {
                    // The filesystem operation has already succeeded. Persist
                    // the recovery marker before reporting the failed mapping.
                    let _ = retro_junk_db::mark_console_stale(conn, console_id);
                    return Err(error);
                }
            }
        }
        R::MarkConsoleStale(id) => {
            LibraryStoreValue::ChangeSet(retro_junk_db::mark_console_stale(conn, id)?)
        }
        R::DeleteConsoleIfEmpty(id) => {
            retro_junk_db::delete_library_console_if_empty(conn, id)?;
            LibraryStoreValue::ChangeSet(LibraryChangeSet::default())
        }
        R::DeleteConsole(id) => {
            retro_junk_db::delete_library_console(conn, id)?;
            LibraryStoreValue::ChangeSet(LibraryChangeSet::default())
        }
        R::DeleteRoot(id) => {
            LibraryStoreValue::ChangeSet(retro_junk_db::delete_library_root(conn, id)?)
        }
        R::DeleteRootPath(root_path) => {
            let changes = match retro_junk_db::get_library_root_id(conn, &root_path)? {
                Some(id) => retro_junk_db::delete_library_root(conn, id)?,
                None => LibraryChangeSet::default(),
            };
            LibraryStoreValue::ChangeSet(changes)
        }
        R::ClearCache => LibraryStoreValue::ChangeSet(retro_junk_db::clear_library_cache(conn)?),
        R::Shutdown => LibraryStoreValue::ShutdownComplete,
    })
}

#[derive(Debug, Clone)]
pub struct Versioned<T> {
    pub revision: u64,
    pub value: T,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum ProjectionStatus {
    #[default]
    Idle,
    Loading {
        request_id: StoreRequestId,
    },
    Failed {
        error: String,
    },
}

#[derive(Debug, Clone, Default)]
pub struct AsyncProjection<T> {
    pub value: Option<Versioned<T>>,
    pub status: ProjectionStatus,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum ProjectionKey {
    Consoles(LibraryRootId),
    List(LibraryEntryListQuery),
    Detail(LibraryEntryId),
}

/// Testable gate between immediate-mode rendering and the store. It rejects
/// old-root, superseded, and older-revision replies and preserves safe prior
/// values while a replacement projection loads.
#[derive(Debug, Default)]
pub struct LibraryProjectionController {
    pub session_generation: UiSessionGeneration,
    next_request_id: StoreRequestId,
    latest: HashMap<ProjectionKey, StoreRequestId>,
    minimum_console_revision: HashMap<LibraryConsoleId, u64>,
    loaded_pages: HashSet<(LibraryEntryListQuery, u64)>,
    pub selected_entries: HashSet<LibraryEntryId>,
    pub focused_entry: Option<LibraryEntryId>,
}

impl LibraryProjectionController {
    pub fn switch_root(&mut self) {
        self.session_generation = self.session_generation.wrapping_add(1);
        self.latest.clear();
        self.loaded_pages.clear();
        self.selected_entries.clear();
        self.focused_entry = None;
    }

    pub fn invalidate_lists(&mut self) {
        self.loaded_pages.clear();
        self.latest
            .retain(|key, _| !matches!(key, ProjectionKey::List(_)));
    }

    pub fn schedule_list(
        &mut self,
        query: LibraryEntryListQuery,
        known_revision: u64,
    ) -> Option<StoreEnvelope<LibraryStoreRequest>> {
        if self.loaded_pages.contains(&(query.clone(), known_revision)) {
            return None;
        }
        let key = ProjectionKey::List(query.clone());
        if self.latest.contains_key(&key) {
            return None;
        }
        Some(self.schedule(key, LibraryStoreRequest::EntryList(query)))
    }

    pub fn schedule_summaries(
        &mut self,
        root: LibraryRootId,
    ) -> StoreEnvelope<LibraryStoreRequest> {
        self.schedule(
            ProjectionKey::Consoles(root),
            LibraryStoreRequest::ConsoleSummaries(root),
        )
    }

    pub fn schedule_detail(&mut self, id: LibraryEntryId) -> StoreEnvelope<LibraryStoreRequest> {
        self.schedule(
            ProjectionKey::Detail(id),
            LibraryStoreRequest::EntryDetail(id),
        )
    }

    fn schedule(
        &mut self,
        key: ProjectionKey,
        payload: LibraryStoreRequest,
    ) -> StoreEnvelope<LibraryStoreRequest> {
        self.next_request_id = self.next_request_id.wrapping_add(1);
        self.latest.insert(key, self.next_request_id);
        StoreEnvelope {
            session_generation: self.session_generation,
            request_id: self.next_request_id,
            payload,
        }
    }

    pub fn accept_list_reply(
        &mut self,
        envelope: &StoreEnvelope<Result<LibraryStoreValue, String>>,
        query: &LibraryEntryListQuery,
    ) -> bool {
        if envelope.session_generation != self.session_generation
            || self.latest.get(&ProjectionKey::List(query.clone())) != Some(&envelope.request_id)
        {
            return false;
        }
        let Ok(LibraryStoreValue::EntryList(page)) = &envelope.payload else {
            self.latest.remove(&ProjectionKey::List(query.clone()));
            return true;
        };
        if page.console_revision
            < self
                .minimum_console_revision
                .get(&query.console_id)
                .copied()
                .unwrap_or(0)
        {
            self.latest.remove(&ProjectionKey::List(query.clone()));
            return false;
        }
        self.loaded_pages
            .insert((query.clone(), page.console_revision));
        self.latest.remove(&ProjectionKey::List(query.clone()));
        true
    }

    pub fn apply_change_set(&mut self, changes: &LibraryChangeSet) {
        if let Some((id, revision)) = changes.console_revision {
            self.minimum_console_revision
                .entry(id)
                .and_modify(|known| *known = (*known).max(revision))
                .or_insert(revision);
            self.loaded_pages.retain(|(query, cached_revision)| {
                query.console_id != id || *cached_revision >= revision
            });
        }
        for id in &changes.removed_entries {
            self.selected_entries.remove(id);
            if self.focused_entry == Some(*id) {
                self.focused_entry = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use retro_junk_db::{LibraryEntryFilter, LibraryEntrySortField, SortDirection};

    fn query() -> LibraryEntryListQuery {
        LibraryEntryListQuery {
            console_id: LibraryConsoleId(7),
            search: String::new(),
            filter: LibraryEntryFilter::All,
            sort: LibraryEntrySortField::DisplayName,
            direction: SortDirection::Ascending,
            offset: 0,
            limit: LibraryEntryListQuery::DEFAULT_PAGE_SIZE,
        }
    }

    fn scanned(name: &str) -> ScannedLibraryEntry {
        ScannedLibraryEntry {
            entry_key: retro_junk_db::file_source_key(std::path::Path::new(name)).unwrap(),
            source_fingerprint: format!("fingerprint:{name}"),
            row: retro_junk_db::LibraryEntryRow {
                display_name: name.into(),
                game_entry_json: format!(r#"{{"SingleFile":"/roms/nes/{name}"}}"#),
                status: "unknown".into(),
                tag: String::new(),
                crc32: String::new(),
                sha1: String::new(),
                md5: String::new(),
                data_size: 0,
                hash_warnings_json: None,
                disc_verification: "not_applicable".into(),
                dat_game_name: String::new(),
                dat_rom_name: String::new(),
                dat_match_method: String::new(),
                region_override: String::new(),
                cover_title: String::new(),
                screen_title: String::new(),
                identification_json: None,
                disc_identifications_json: None,
                broken_references_json: None,
                ambiguous_candidates_json: None,
                cue_compat_issues_json: None,
            },
        }
    }

    #[test]
    fn unchanged_frames_schedule_zero_additional_reads() {
        let mut controller = LibraryProjectionController::default();
        let request = controller.schedule_list(query(), 4).unwrap();
        let reply = StoreEnvelope {
            session_generation: 0,
            request_id: request.request_id,
            payload: Ok(LibraryStoreValue::EntryList(LibraryEntryListPage {
                console_id: LibraryConsoleId(7),
                console_revision: 4,
                total_count: 0,
                logical_count: 0,
                counts: Default::default(),
                availability_counts: Default::default(),
                archived_playable_gaps: Vec::new(),
                archived_releases: Vec::new(),
                offset: 0,
                rows: Vec::new(),
            })),
        };
        assert!(controller.accept_list_reply(&reply, &query()));
        assert!(controller.schedule_list(query(), 4).is_none());
    }

    #[test]
    fn root_session_and_revision_reject_stale_replies() {
        let mut controller = LibraryProjectionController::default();
        let request = controller.schedule_list(query(), 1).unwrap();
        controller.apply_change_set(&LibraryChangeSet {
            console_revision: Some((LibraryConsoleId(7), 3)),
            ..Default::default()
        });
        let stale = StoreEnvelope {
            session_generation: 0,
            request_id: request.request_id,
            payload: Ok(LibraryStoreValue::EntryList(LibraryEntryListPage {
                console_id: LibraryConsoleId(7),
                console_revision: 2,
                total_count: 0,
                logical_count: 0,
                counts: Default::default(),
                availability_counts: Default::default(),
                archived_playable_gaps: Vec::new(),
                archived_releases: Vec::new(),
                offset: 0,
                rows: Vec::new(),
            })),
        };
        assert!(!controller.accept_list_reply(&stale, &query()));
        controller.switch_root();
        assert!(!controller.accept_list_reply(&stale, &query()));
    }

    #[test]
    fn removed_ids_are_cleaned_from_selection_and_focus() {
        let mut controller = LibraryProjectionController::default();
        controller
            .selected_entries
            .extend([LibraryEntryId(1), LibraryEntryId(2)]);
        controller.focused_entry = Some(LibraryEntryId(2));
        controller.apply_change_set(&LibraryChangeSet {
            removed_entries: vec![LibraryEntryId(2)],
            ..Default::default()
        });
        assert_eq!(
            controller.selected_entries,
            HashSet::from([LibraryEntryId(1)])
        );
        assert_eq!(controller.focused_entry, None);
    }

    #[test]
    fn change_invalidates_only_its_console_projection() {
        let mut controller = LibraryProjectionController::default();
        let q1 = query();
        let mut q2 = query();
        q2.console_id = LibraryConsoleId(8);
        for query in [&q1, &q2] {
            let request = controller.schedule_list(query.clone(), 1).unwrap();
            let reply = StoreEnvelope {
                session_generation: 0,
                request_id: request.request_id,
                payload: Ok(LibraryStoreValue::EntryList(LibraryEntryListPage {
                    console_id: query.console_id,
                    console_revision: 1,
                    total_count: 0,
                    logical_count: 0,
                    counts: Default::default(),
                    availability_counts: Default::default(),
                    archived_playable_gaps: Vec::new(),
                    archived_releases: Vec::new(),
                    offset: 0,
                    rows: Vec::new(),
                })),
            };
            assert!(controller.accept_list_reply(&reply, query));
        }
        controller.apply_change_set(&LibraryChangeSet {
            console_revision: Some((q1.console_id, 2)),
            ..Default::default()
        });
        assert!(controller.schedule_list(q1, 2).is_some());
        assert!(controller.schedule_list(q2, 1).is_none());
    }

    #[test]
    fn store_reads_and_joins_both_workers_after_shutdown() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let conn = retro_junk_db::open_database(&path).unwrap();
        let root = retro_junk_db::upsert_library_root(&conn, "/roms").unwrap();
        drop(conn);

        let mut store = LibraryStore::start(path).unwrap();
        store
            .submit(StoreEnvelope {
                session_generation: 8,
                request_id: 42,
                payload: LibraryStoreRequest::ConsoleSummaries(root),
            })
            .unwrap();
        let reply = store
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert_eq!(reply.session_generation, 8);
        assert_eq!(reply.request_id, 42);
        assert!(matches!(
            reply.payload,
            Ok(LibraryStoreValue::ConsoleSummaries(ref rows)) if rows.is_empty()
        ));
        store.shutdown_and_join(8);
        assert!(store.threads.is_empty());
    }

    #[test]
    fn worker_loads_every_console_detail_without_pagination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let conn = retro_junk_db::open_database(&path).unwrap();
        let root = retro_junk_db::upsert_library_root(&conn, "/roms").unwrap();
        let console = retro_junk_db::ensure_library_console(
            &conn,
            &LibraryConsoleDescriptor {
                root_id: root,
                platform: "Genesis".into(),
                folder_name: "genesis".into(),
                folder_path: "/roms/genesis".into(),
            },
        )
        .unwrap();
        for index in 0..350 {
            let entry_key = format!("file:{index}.md");
            let display_name = format!("{index}.md");
            let game_entry_json = format!(r#"{{"SingleFile":"/roms/genesis/{index}.md"}}"#);
            conn.execute(
                "INSERT INTO library_entries(console_id,entry_key,display_name,game_entry_json)
                 VALUES(?1,?2,?3,?4)",
                (console.0, entry_key, display_name, game_entry_json),
            )
            .unwrap();
        }
        drop(conn);

        let mut store = LibraryStore::start(path).unwrap();
        store
            .submit(StoreEnvelope {
                session_generation: 3,
                request_id: 9,
                payload: LibraryStoreRequest::ConsoleEntryDetails(console),
            })
            .unwrap();
        let reply = store
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        assert!(matches!(
            reply.payload,
            Ok(LibraryStoreValue::ConsoleDetails {
                ref entries,
                ref archived_releases,
            }) if entries.len() == 350 && archived_releases.is_empty()
        ));
        store.shutdown_and_join(3);
    }

    #[test]
    fn committed_scan_survives_immediate_shutdown_and_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let mut store = LibraryStore::start(path.clone()).unwrap();
        store
            .submit(StoreEnvelope {
                session_generation: 1,
                request_id: 1,
                payload: LibraryStoreRequest::CommitConsoleScan(CompletedConsoleScan {
                    root_path: "/roms".into(),
                    platform: "Nes".into(),
                    folder_name: "nes".into(),
                    folder_path: "/roms/nes".into(),
                    fingerprint_hash: "folder".into(),
                    entries: vec![scanned("game.nes")],
                }),
            })
            .unwrap();
        store.shutdown_and_join(1);

        let conn = retro_junk_db::open_database(&path).unwrap();
        let root = retro_junk_db::get_library_root_id(&conn, "/roms")
            .unwrap()
            .unwrap();
        let summaries = retro_junk_db::list_console_summaries(&conn, root).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].entry_count, 1);
    }

    #[test]
    fn failed_filesystem_transition_marks_console_stale_before_reply() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("catalog.db");
        let mut store = LibraryStore::start(path.clone()).unwrap();
        store
            .submit(StoreEnvelope {
                session_generation: 1,
                request_id: 1,
                payload: LibraryStoreRequest::CommitConsoleScan(CompletedConsoleScan {
                    root_path: "/roms".into(),
                    platform: "Nes".into(),
                    folder_name: "nes".into(),
                    folder_path: "/roms/nes".into(),
                    fingerprint_hash: "folder".into(),
                    entries: vec![scanned("game.nes")],
                }),
            })
            .unwrap();
        let first = store
            .recv_timeout(std::time::Duration::from_secs(2))
            .unwrap();
        let (console_id, entry_id) = match first.payload.unwrap() {
            LibraryStoreValue::ConsoleScanCommitted {
                console_id,
                changes,
                ..
            } => (console_id, changes.affected_entries[0]),
            other => panic!("unexpected reply: {other:?}"),
        };
        store
            .submit(StoreEnvelope {
                session_generation: 1,
                request_id: 2,
                payload: LibraryStoreRequest::ApplyFilesystemTransition {
                    console_id,
                    entry_id,
                    expected_source_revision: 99,
                    entry: scanned("renamed.nes"),
                },
            })
            .unwrap();
        assert!(
            store
                .recv_timeout(std::time::Duration::from_secs(2))
                .unwrap()
                .payload
                .is_err()
        );
        store.shutdown_and_join(1);

        let conn = retro_junk_db::open_database(&path).unwrap();
        let root = retro_junk_db::get_library_root_id(&conn, "/roms")
            .unwrap()
            .unwrap();
        assert_eq!(
            retro_junk_db::list_console_summaries(&conn, root).unwrap()[0].scan_state,
            retro_junk_db::LibraryScanState::Stale
        );
    }
}
