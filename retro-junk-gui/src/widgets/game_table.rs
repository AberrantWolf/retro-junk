use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::path::PathBuf;

use egui_table::{CellInfo, Column, HeaderCellInfo, HeaderRow, Table, TableDelegate};
use retro_junk_lib::Region;

use crate::app::RetroJunkApp;
use crate::backend;
use crate::state::{AssetStatus, EntryStatus, FocusedPanel, TagDialog};
use crate::util;
use crate::widgets::icons;
use crate::widgets::keyboard_nav;
use crate::widgets::status_badge;

/// Height of one data row and of the sticky header row.
const ROW_HEIGHT: f32 = 20.0;

/// Columns that are always present, followed by the per-analyzer optional
/// ones. `ColumnKind` is the single description of a column: its header,
/// its default/minimum width, and the cell it renders. Adding a column
/// means adding a variant, not touching four parallel lists.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColumnKind {
    Status,
    Name,
    Availability,
    InternalName,
    Region,
    DatMatch,
}

impl ColumnKind {
    fn header(self) -> &'static str {
        match self {
            Self::Status => "",
            Self::Name => "Name",
            Self::Availability => "Availability",
            Self::InternalName => "Internal Name",
            Self::Region => "Region",
            Self::DatMatch => "DAT Match",
        }
    }

    /// `(initial width, minimum width)`; the status column is fixed.
    fn width(self) -> (f32, f32) {
        match self {
            Self::Status => (30.0, 30.0),
            Self::Name => (280.0, 100.0),
            Self::Availability => (145.0, 90.0),
            Self::InternalName => (140.0, 60.0),
            Self::Region => (80.0, 40.0),
            Self::DatMatch => (200.0, 80.0),
        }
    }

    fn column(self) -> Column {
        let (initial, minimum) = self.width();
        Column::new(initial)
            .range(minimum..=1200.0)
            .resizable(self != Self::Status)
    }
}

/// Render the sortable, filterable game table for the selected console.
pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(console_idx) = app.selected_console_index() else {
        return;
    };

    let console = &app.browser.consoles[console_idx];
    let Some(console_id) = console.id else { return };
    let list_columns = app.context.get_by_platform(console.platform).map_or_else(
        EntryListColumns::default,
        |registered| EntryListColumns {
            internal_name: registered.metadata.identification.internal_name
                && registered.analyzer.dat_source() != retro_junk_lib::DatSource::Redump,
            region: registered.metadata.identification.region,
            dat_match: registered.analyzer.has_dat_support(),
        },
    );
    let Some(page) = app
        .browser
        .active_page
        .as_ref()
        .filter(|page| page.console_id == console_id)
    else {
        ui.label("Loading library entries…");
        return;
    };
    let visible_ids: Vec<_> = page
        .rows
        .iter()
        .filter(|row| row.archive_release_id.is_none())
        .map(|row| row.id)
        .collect();

    // Cmd+A / Ctrl+A: select all visible entries
    if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, egui::Key::A)) {
        app.ui_state.selected_entries = visible_ids.iter().copied().collect();
        app.ui_state.selected_archive_releases.clear();
    }

    // Keyboard navigation (only when game table has focus)
    if app.ui_state.focused_panel == FocusedPanel::GameTable {
        let current_filtered_pos = visible_ids
            .iter()
            .position(|id| Some(*id) == app.ui_state.focused_entry);

        let text_height_estimate = egui::TextStyle::Body
            .resolve(ui.style())
            .size
            .max(ui.spacing().interact_size.y);
        let page_size = (ui.available_height() / text_height_estimate).max(1.0) as usize;

        if let Some(action) =
            keyboard_nav::process_list_nav(ui, current_filtered_pos, visible_ids.len(), page_size)
        {
            let entry_id = visible_ids[action.new_index];

            if action.extend_selection {
                // Shift: extend selection from focused to new position
                if let Some(focused_pos) = current_filtered_pos {
                    let start = focused_pos.min(action.new_index);
                    let end = focused_pos.max(action.new_index);
                    for &id in &visible_ids[start..=end] {
                        app.ui_state.selected_entries.insert(id);
                    }
                } else {
                    app.ui_state.selected_entries.insert(entry_id);
                }
            } else {
                app.ui_state.selected_entries.clear();
                app.ui_state.selected_archive_releases.clear();
                app.ui_state.selected_entries.insert(entry_id);
            }

            app.ui_state.focused_entry = Some(entry_id);
            app.ui_state.scroll_to_row = Some(action.new_index);
        }
    }

    // Status summary
    let total = page.logical_count;
    let matched = page.counts.matched;
    let likely = page.counts.likely;
    let ambiguous = page.counts.ambiguous;
    let unrecognized = page.counts.unrecognized;
    let tagged = page.counts.tagged;
    let archive_release_count = page
        .availability_counts
        .archived_and_playable
        .saturating_add(page.availability_counts.incomplete_archive_and_playable)
        .saturating_add(page.availability_counts.preferred_format_mismatch)
        .saturating_add(page.availability_counts.archived_not_playable);

    // Rich rows are sparse (focus/selection only). Index them once so a large
    // selected page does not turn projection building into O(page × details).
    let entry_lookup: HashMap<_, _> = console
        .entries
        .iter()
        .filter_map(|entry| entry.id.map(|id| (id, entry)))
        .collect();

    // Pre-extract row data to avoid borrowing issues
    let mut row_data: Vec<RowData> = page
        .archived_releases
        .iter()
        .map(|release| {
            let summary = &release.summary;
            let bound_entry_ids = release
                .playable_library_entries
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>();
            RowData {
                entry_id: None,
                bound_entry_ids: bound_entry_ids.clone(),
                archive_release_id: Some(summary.archive_release_id.clone()),
                status: if summary.archive_complete {
                    EntryStatus::Matched
                } else {
                    EntryStatus::Unknown
                },
                has_broken_refs: false,
                has_hash_warnings: false,
                has_cue_compat_issues: false,
                asset_status: aggregate_asset_status(
                    bound_entry_ids
                        .iter()
                        .filter_map(|id| app.browser.asset_statuses.get(id).copied())
                        .chain(std::iter::once(archive_asset_status(
                            &release.archived_assets,
                        ))),
                ),
                name: summary.title.clone(),
                file_path: bound_entry_ids.iter().find_map(|id| {
                    entry_lookup
                        .get(id)
                        .map(|entry| entry.game_entry.analysis_path().to_path_buf())
                }),
                internal_name: String::new(),
                regions: region_code(&summary.region),
                dat_match: summary.catalog_release_id.clone().unwrap_or_default(),
                availability: AvailabilityState::from_archive(release),
            }
        })
        .collect();
    row_data.extend(
        page.rows
            .iter()
            // A catalog-bound playable entry is one representation of the logical
            // archive release row above, not a second game in the user's Library.
            .filter(|projection| projection.archive_release_id.is_none())
            .map(|projection| {
                let entry = entry_lookup.get(&projection.id).copied();
                RowData {
                    entry_id: Some(projection.id),
                    bound_entry_ids: Vec::new(),
                    archive_release_id: None,
                    status: projection_status(&projection.status, &projection.tag),
                    has_broken_refs: projection.has_broken_references,
                    has_hash_warnings: projection.has_hash_warnings,
                    has_cue_compat_issues: projection.has_cue_compat_issues,
                    asset_status: app
                        .browser
                        .asset_statuses
                        .get(&projection.id)
                        .copied()
                        .unwrap_or(AssetStatus::Unknown),
                    name: projection.display_name.clone(),
                    file_path: entry.map(|entry| entry.game_entry.analysis_path().to_path_buf()),
                    internal_name: projection.internal_name.clone(),
                    regions: projected_regions(projection),
                    dat_match: projection.dat_game_name.clone(),
                    availability: AvailabilityState::from_projection(projection),
                }
            }),
    );
    row_data.sort_by(|left, right| {
        left.name
            .to_ascii_lowercase()
            .cmp(&right.name.to_ascii_lowercase())
            .then_with(|| left.regions.cmp(&right.regions))
    });

    ui.horizontal(|ui| {
        let mut summary = format!(
            "{total} games | {archive_release_count} archived releases | {matched} playable files verified | {likely} likely | {ambiguous} ambiguous | {unrecognized} unrecognized"
        );
        if tagged > 0 {
            let _ = write!(summary, " | {tagged} tagged");
        }
        let _ = write!(summary, " | showing {} logical rows", row_data.len());
        ui.label(summary);
    });

    ui.add_space(2.0);

    let mut columns = vec![
        ColumnKind::Status,
        ColumnKind::Name,
        ColumnKind::Availability,
    ];
    if list_columns.internal_name {
        columns.push(ColumnKind::InternalName);
    }
    if list_columns.region {
        columns.push(ColumnKind::Region);
    }
    if list_columns.dat_match {
        columns.push(ColumnKind::DatMatch);
    }

    // The column set changes per console, and `egui_table` keys persisted
    // widths by column index. Salting the table id with the set keeps a
    // six-column console's widths from being applied to a four-column one.
    let id_salt = columns
        .iter()
        .map(|kind| kind.header())
        .collect::<Vec<_>>()
        .join("|");
    let mut table = Table::new()
        .id_salt(("game_table", id_salt))
        .num_rows(row_data.len() as u64)
        .columns(columns.iter().map(|kind| kind.column()).collect::<Vec<_>>())
        // Badge and name stay put while the wider identification columns
        // scroll horizontally.
        .num_sticky_cols(2)
        .headers(vec![HeaderRow::new(ROW_HEIGHT)]);
    if let Some(row) = app.ui_state.scroll_to_row.take() {
        table = table.scroll_to_row(row as u64, Some(egui::Align::Center));
    }

    let mut delegate = GameTableDelegate {
        app,
        ctx,
        console_idx,
        columns,
        rows: row_data,
        visible_ids,
    };
    table.show(ui, &mut delegate);

    delegate.app.request_selected_entry_details(ctx);
}

/// Bridges the derived row projection to `egui_table`'s pull-based
/// rendering. It owns no logic of its own: every click path here is the
/// same selection/menu behaviour the table had before, moved from a
/// unioned per-column response onto the row `Ui` the crate already builds.
struct GameTableDelegate<'a> {
    app: &'a mut RetroJunkApp,
    ctx: &'a egui::Context,
    console_idx: usize,
    columns: Vec<ColumnKind>,
    rows: Vec<RowData>,
    visible_ids: Vec<retro_junk_db::LibraryEntryId>,
}

impl TableDelegate for GameTableDelegate<'_> {
    fn header_cell_ui(&mut self, ui: &mut egui::Ui, cell: &HeaderCellInfo) {
        let Some(kind) = self.columns.get(cell.col_range.start) else {
            return;
        };
        cell_frame(ui, |ui| {
            ui.strong(kind.header());
        });
    }

    fn row_ui(&mut self, ui: &mut egui::Ui, row_nr: u64) {
        let Some(data) = self.rows.get(row_nr as usize) else {
            return;
        };
        let is_selected = data
            .entry_id
            .is_some_and(|id| self.app.ui_state.selected_entries.contains(&id))
            || data
                .archive_release_id
                .as_ref()
                .is_some_and(|id| self.app.ui_state.selected_archive_releases.contains(id));
        let is_focused = row_is_focused(
            data.entry_id,
            data.archive_release_id.as_deref(),
            self.app.ui_state.focused_entry,
            self.app.ui_state.focused_archive_release.as_deref(),
        );

        if is_selected || is_focused {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, ui.visuals().selection.bg_fill);
        } else if row_nr % 2 == 1 {
            ui.painter()
                .rect_filled(ui.max_rect(), 0.0, ui.visuals().faint_bg_color);
        }

        if !row_supports_context_menu(data.entry_id, data.archive_release_id.as_deref()) {
            return;
        }
        // With sticky columns the crate renders each row twice — once for the
        // frozen columns, once for the scrolling ones. Each row `Ui` is
        // clipped to its own region, so at most one of the two responses can
        // contain the pointer and no de-duplication is needed here.
        let response = ui.response().interact(egui::Sense::click());

        // Both the click handlers and the context menu need `self.app`
        // mutably, so take an owned copy of the row up front rather than
        // holding a borrow of `self.rows` across them.
        let data = data.clone();
        if response.secondary_clicked() {
            self.select_for_context_menu(&data);
        } else if response.clicked() {
            self.handle_primary_click(&data);
        }

        let console_idx = self.console_idx;
        let app = &mut *self.app;
        let ctx = self.ctx;
        response.context_menu(|ui| {
            show_row_context_menu(ui, app, ctx, console_idx, &data);
        });
    }

    fn cell_ui(&mut self, ui: &mut egui::Ui, cell: &CellInfo) {
        let (Some(kind), Some(data)) = (
            self.columns.get(cell.col_nr).copied(),
            self.rows.get(cell.row_nr as usize),
        ) else {
            return;
        };
        cell_frame(ui, |ui| match kind {
            ColumnKind::Status => show_status_cell(ui, data),
            ColumnKind::Name => paint_cell_text(ui, &data.name),
            ColumnKind::Availability => {
                paint_cell_text(ui, data.availability.label());
                ui.response()
                    .on_hover_text_at_pointer(data.availability.tooltip());
            }
            ColumnKind::InternalName => paint_cell_text(ui, &data.internal_name),
            ColumnKind::Region => paint_cell_text(ui, &data.regions),
            ColumnKind::DatMatch => paint_cell_text(ui, &data.dat_match),
        });
    }

    fn default_row_height(&self) -> f32 {
        ROW_HEIGHT
    }
}

impl GameTableDelegate<'_> {
    fn handle_primary_click(&mut self, data: &RowData) {
        let modifiers = self.ctx.input(|i| i.modifiers);
        if let Some(entry_id) = data.entry_id {
            handle_row_click(self.app, entry_id, modifiers, &self.visible_ids);
            if !self.app.ui_state.selected_entries.contains(&entry_id)
                && self.app.selected_library_row_count() == 1
                && let Some(release_id) = self
                    .app
                    .ui_state
                    .selected_archive_releases
                    .iter()
                    .next()
                    .cloned()
            {
                self.app.ui_state.focused_entry = None;
                self.app.ui_state.focused_archive_release = Some(release_id);
            } else {
                self.app.ui_state.focused_archive_release = None;
                if let Some(focused) = self.app.ui_state.focused_entry {
                    self.app.request_entry_detail(focused, self.ctx);
                }
            }
        } else if let Some(release_id) = data.archive_release_id.as_ref() {
            select_archive_release(
                self.app,
                release_id,
                &data.bound_entry_ids,
                modifiers,
                self.ctx,
            );
        }
        self.app.ui_state.focused_panel = FocusedPanel::GameTable;
    }

    /// A right-click on an unselected row selects just that row, so the menu
    /// always acts on what the user pointed at.
    fn select_for_context_menu(&mut self, data: &RowData) {
        if let Some(entry_id) = data.entry_id {
            if !self.app.ui_state.selected_entries.contains(&entry_id) {
                self.app.ui_state.selected_entries.clear();
                self.app.ui_state.selected_archive_releases.clear();
                self.app.ui_state.selected_entries.insert(entry_id);
            }
            self.app.ui_state.focused_entry = Some(entry_id);
            self.app.ui_state.focused_archive_release = None;
            self.app.request_entry_detail(entry_id, self.ctx);
        } else if let Some(release_id) = data.archive_release_id.as_ref() {
            if self
                .app
                .ui_state
                .selected_archive_releases
                .contains(release_id)
            {
                self.app.ui_state.focused_entry = None;
                self.app.ui_state.focused_archive_release = Some(release_id.clone());
            } else {
                select_archive_release(
                    self.app,
                    release_id,
                    &data.bound_entry_ids,
                    egui::Modifiers::NONE,
                    self.ctx,
                );
            }
        }
    }
}

/// Uniform horizontal padding inside every header and data cell.
fn cell_frame<R>(ui: &mut egui::Ui, contents: impl FnOnce(&mut egui::Ui) -> R) {
    egui::Frame::NONE
        .inner_margin(egui::Margin::symmetric(4, 0))
        .show(ui, contents);
}

fn show_status_cell(ui: &mut egui::Ui, data: &RowData) {
    let response = status_badge::show_with_warning(
        ui,
        data.status,
        data.has_broken_refs,
        data.has_hash_warnings,
        data.has_cue_compat_issues,
        data.asset_status,
    );
    let mut tip = data.status.tooltip().to_string();
    if data.has_broken_refs {
        let _ = write!(tip, "\n{} Broken file references", icons::WARNING);
    }
    if data.has_hash_warnings {
        let _ = write!(tip, "\n{} Hash warnings (see detail panel)", icons::WARNING);
    }
    if data.has_cue_compat_issues {
        let _ = write!(tip, "\n{} Non-standard CUE sheet format", icons::WARNING);
    }
    match data.asset_status {
        AssetStatus::Unknown => {}
        AssetStatus::None => tip.push_str("\nNo scraped media"),
        AssetStatus::Partial { found, total } => {
            let _ = write!(tip, "\nPartial media ({found}/{total})");
        }
        AssetStatus::Complete => tip.push_str("\nMedia complete"),
    }
    response.on_hover_text(tip);
}

/// Render the context menu for a game table row.
fn show_row_context_menu(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    ctx: &egui::Context,
    console_idx: usize,
    data: &RowData,
) {
    if app.selected_library_row_count() > 1 {
        show_multi_row_context_menu(ui, app, ctx, console_idx);
        return;
    }
    let archive_release = data.archive_release_id.as_ref().and_then(|release_id| {
        app.browser
            .active_page
            .as_ref()?
            .archived_releases
            .iter()
            .find(|release| &release.summary.archive_release_id == release_id)
            .cloned()
    });
    if let Some(release) = archive_release.as_ref() {
        show_archive_context_menu(ui, app, ctx, console_idx, release);
        if data.bound_entry_ids.is_empty() {
            return;
        }
        ui.separator();
    }

    if !app.selected_entry_details_loaded() {
        ui.spinner();
        ui.label("Loading the complete selection…");
        return;
    }
    let Some(file_path) = data.file_path.as_ref() else {
        ui.label("Loading entry details…");
        return;
    };
    if ui.button(icons::labeled(icons::RESCAN, "Rescan")).clicked() {
        backend::scan::rescan_selected_entries(app, console_idx, ctx);
        ui.close();
    }

    if ui
        .button(icons::labeled(icons::HASH, "Calculate Missing Hashes"))
        .clicked()
    {
        backend::hash::compute_hashes_for_selection(app, console_idx);
        ui.close();
    }
    if ui
        .button(icons::labeled(icons::HASH, "Recalculate Hashes"))
        .clicked()
    {
        backend::hash::recompute_hashes_for_selection(app, console_idx);
        ui.close();
    }

    // Adaptive scrape menu items based on aggregate media state
    if archive_release.is_none() {
        let console = &app.browser.consoles[console_idx];
        let mut any_has_media = false;
        let mut all_have_all_media = true;
        let mut any_has_miximage = false;
        for &i in &app.ui_state.selected_entries {
            if console.entry_by_id(i).is_some() {
                let ms = app
                    .browser
                    .asset_statuses
                    .get(&i)
                    .copied()
                    .unwrap_or(AssetStatus::Unknown);
                match ms {
                    AssetStatus::Complete => {
                        any_has_media = true;
                    }
                    AssetStatus::Partial { .. } => {
                        any_has_media = true;
                        all_have_all_media = false;
                    }
                    _ => {
                        all_have_all_media = false;
                    }
                }
                if app.browser.entries_with_miximages.contains(&i) {
                    any_has_miximage = true;
                }
            }
        }

        if any_has_media && all_have_all_media {
            // All entries have complete media
            if ui
                .button(icons::labeled(icons::SCRAPE, "Re-scrape Media"))
                .clicked()
            {
                backend::assets::rescrape_media_for_selection(app, console_idx, ctx);
                ui.close();
            }
        } else if any_has_media {
            // Some entries have partial media
            if ui
                .button(icons::labeled(icons::SCRAPE, "Scrape All Media"))
                .clicked()
            {
                backend::assets::rescrape_media_for_selection(app, console_idx, ctx);
                ui.close();
            }
            if ui
                .button(icons::labeled(icons::SCRAPE, "Scrape Missing Media"))
                .clicked()
            {
                backend::assets::scrape_missing_media_for_selection(app, console_idx, ctx);
                ui.close();
            }
        } else {
            // No entry has any media
            if ui
                .button(icons::labeled(icons::SCRAPE, "Scrape Media"))
                .clicked()
            {
                backend::assets::rescrape_media_for_selection(app, console_idx, ctx);
                ui.close();
            }
        }

        // Miximage menu items (hidden when no media at all — scraping will auto-generate)
        if any_has_media {
            let label = if any_has_miximage {
                "Re-generate Miximages"
            } else {
                "Generate Miximages"
            };
            if ui.button(icons::labeled(icons::MIXIMAGE, label)).clicked() {
                backend::assets::regenerate_miximages_for_selection(app, console_idx, ctx);
                ui.close();
            }
        }
    }

    // Archive-derived playable files are named and tracked by immutable build
    // evidence. Mutating their names/layout through ordinary library tools
    // would stale that evidence; use the release-level build action instead.
    if data.archive_release_id.is_none() {
        if ui
            .button(icons::labeled(icons::RENAME, "Auto Rename"))
            .clicked()
        {
            backend::rename::rename_selected_entries(app, console_idx, ctx);
            ui.close();
        }

        // Fix CUE Sheet — only show when at least one selected entry has CUE compat issues
        {
            let console = &app.browser.consoles[console_idx];
            let has_cue_issues = app.ui_state.selected_entries.iter().any(|&i| {
                console
                    .entry_by_id(i)
                    .is_some_and(super::super::state::LibraryEntry::has_cue_compat_issues)
            });
            if has_cue_issues
                && ui
                    .button(icons::labeled(icons::FIX_CUE, "Fix CUE Sheet"))
                    .clicked()
            {
                backend::fix_cue::fix_cue_for_selection(app, console_idx, ctx);
                ui.close();
            }
        }

        // Compress to CHD — only for consoles whose analyzer supports it. The
        // dialog itself explains per-disc skips and a missing chdman. Gated
        // (advisory) while a compression is already running for this console.
        if backend::chd_compress::console_supports_chd(app, console_idx) {
            let busy = app.chd_compress_busy(&app.browser.consoles[console_idx].folder_name);
            let button = ui
                .add_enabled(
                    !busy,
                    egui::Button::new(icons::labeled(icons::COMPRESS, "Compress to CHD…")),
                )
                .on_disabled_hover_text("A CHD compression is already running for this console");
            if button.clicked() {
                let selection = app.selected_entry_indices();
                backend::chd_compress::open_compress_dialog(app, console_idx, &selection);
                ui.close();
            }
        }

        // Region overrides and tags belong to standalone playable entries,
        // not to the authoritative archive release.
        show_set_region_submenu(ui, app, console_idx);
        if app.ui_state.selected_entries.len() == 1 {
            let entry_id = *app.ui_state.selected_entries.iter().next().unwrap();
            show_tag_menu_items(ui, app, console_idx, entry_id);
        }
    }

    ui.separator();

    if ui
        .button(icons::labeled(icons::REVEAL, util::REVEAL_LABEL))
        .clicked()
    {
        util::reveal_in_file_manager(file_path);
        ui.close();
    }

    ui.separator();

    if ui
        .button(icons::labeled(icons::COPY, "Copy File Path"))
        .clicked()
    {
        let paths = collect_selected_field(app, console_idx, |entry| {
            Some(entry.game_entry.analysis_path().display().to_string())
        });
        crate::util::copy_and_close(ui, paths);
    }

    let has_serial = app.ui_state.selected_entries.iter().any(|&i| {
        app.browser.consoles[console_idx]
            .entry_by_id(i)
            .and_then(|e| e.identification.as_ref())
            .is_some_and(|id| !id.serial_number.is_empty())
    });
    if ui
        .add_enabled(
            has_serial,
            egui::Button::new(icons::labeled(icons::COPY, "Copy Serial")),
        )
        .clicked()
    {
        let serials = collect_selected_field(app, console_idx, |entry| {
            entry
                .identification
                .as_ref()
                .filter(|id| !id.serial_number.is_empty())
                .map(|id| id.serial_number.clone())
        });
        crate::util::copy_and_close(ui, serials);
    }

    let has_crc32 = app.ui_state.selected_entries.iter().any(|&i| {
        app.browser.consoles[console_idx]
            .entry_by_id(i)
            .and_then(|e| e.hashes.as_ref())
            .is_some()
    });
    if ui
        .add_enabled(
            has_crc32,
            egui::Button::new(icons::labeled(icons::COPY, "Copy CRC32")),
        )
        .clicked()
    {
        let crcs = collect_selected_field(app, console_idx, |entry| {
            entry.hashes.as_ref().map(|h| h.crc32.clone())
        });
        crate::util::copy_and_close(ui, crcs);
    }

    let has_dat = !data.dat_match.is_empty()
        || app.ui_state.selected_entries.iter().any(|&i| {
            app.browser.consoles[console_idx]
                .entry_by_id(i)
                .and_then(|e| e.dat_match.as_ref())
                .is_some()
        });
    if ui
        .add_enabled(
            has_dat,
            egui::Button::new(icons::labeled(icons::COPY, "Copy DAT Name")),
        )
        .clicked()
    {
        let dat_names = collect_selected_field(app, console_idx, |entry| {
            entry.dat_match.as_ref().map(|dm| dm.game_name.clone())
        });
        crate::util::copy_and_close(ui, dat_names);
    }
}

fn show_archive_context_menu(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    ctx: &egui::Context,
    console_idx: usize,
    release: &retro_junk_db::ArchivedLibraryListItem,
) {
    let scrape = ui
        .add_enabled(
            release.scrape_identity.is_some(),
            egui::Button::new(icons::labeled(icons::SCRAPE, "Scrape Missing Artwork")),
        )
        .on_disabled_hover_text("This release has no reliable catalog scraper identity");
    if scrape.clicked() {
        backend::assets::scrape_missing_artwork_for_selection(app, console_idx, ctx);
        ui.close();
    }
    if ui
        .button(icons::labeled(icons::MIXIMAGE, "Generate Miximage"))
        .clicked()
    {
        backend::assets::regenerate_miximages_for_selection(app, console_idx, ctx);
        ui.close();
    }

    let frontend_stems = release
        .playable_library_entries
        .iter()
        .filter_map(|entry| {
            let name = entry.display_name.as_str();
            if name.to_ascii_lowercase().ends_with(".m3u") {
                Some(name.to_owned())
            } else {
                std::path::Path::new(name)
                    .file_stem()
                    .and_then(|value| value.to_str())
                    .map(str::to_owned)
            }
        })
        .collect::<Vec<_>>();
    let has_frontend_target = !release.playable_library_entries.is_empty()
        || !release.playable_representations.is_empty();
    let restore = ui
        .add_enabled(
            has_frontend_target && !release.archived_assets.is_empty(),
            egui::Button::new(icons::labeled(icons::SCRAPE, "Restore Archived Media")),
        )
        .on_disabled_hover_text(if release.archived_assets.is_empty() {
            "This release has no archived media"
        } else {
            "Make or adopt a playable representation first"
        });
    if restore.clicked() {
        backend::assets::restore_archived_media_for_release(
            app,
            release.summary.archive_release_id.clone(),
            app.browser.consoles[console_idx].folder_name.clone(),
            frontend_stems,
        );
        ui.close();
    }

    if let Some(action) = release.action.as_ref() {
        ui.separator();
        let needs_verification = action
            .carriers
            .iter()
            .any(|carrier| !carrier.catalog_verified);
        let (icon, label) = if action.needs_playlist && !action.needs_playable {
            (icons::BUILD_PLAYABLE, "Create Multi-disc Playlist")
        } else if !action.needs_playable {
            (icons::VERIFY, "Verify Archive")
        } else if needs_verification && !action.allow_unverified {
            (icons::VERIFY, "Verify & Make Playable")
        } else {
            (icons::BUILD_PLAYABLE, "Make Playable")
        };
        let format = action
            .preferred_format
            .as_deref()
            .and_then(parse_archive_playable_format)
            .or_else(|| {
                (!action.needs_playable).then_some(retro_junk_archive::RepresentationFormat::Rom)
            });
        let build = ui
            .add_enabled(
                action.buildable && format.is_some(),
                egui::Button::new(icons::labeled(icon, label)),
            )
            .on_disabled_hover_text(
                if action.preferred_format.is_none() && action.needs_playable {
                    "Choose a preferred playable format for this console first"
                } else {
                    "The archived carrier set is incomplete or has no supported conversion"
                },
            );
        if build.clicked()
            && let Some(format) = format
        {
            backend::playable_build::start(
                app,
                action.clone(),
                &format,
                app.browser.consoles[console_idx].folder_name.clone(),
                ctx,
            );
            ui.close();
        }
    }
}

fn show_multi_row_context_menu(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    ctx: &egui::Context,
    console_idx: usize,
) {
    ui.strong(format!(
        "{} items selected",
        app.selected_library_row_count()
    ));
    let selected_releases = app
        .browser
        .active_page
        .as_ref()
        .map(|page| {
            page.archived_releases
                .iter()
                .filter(|release| {
                    app.ui_state
                        .selected_archive_releases
                        .contains(&release.summary.archive_release_id)
                })
                .cloned()
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let details_ready = app.selected_entry_details_loaded();
    if !details_ready {
        ui.spinner();
        ui.label("Loading the complete selection…");
    }

    let scrape_ready = details_ready
        && selected_releases
            .iter()
            .all(|release| release.scrape_identity.is_some());
    if ui
        .add_enabled(
            scrape_ready,
            egui::Button::new(icons::labeled(icons::SCRAPE, "Scrape Only Missing Artwork")),
        )
        .clicked()
    {
        backend::assets::scrape_missing_artwork_for_selection(app, console_idx, ctx);
        ui.close();
    }
    if ui
        .add_enabled(
            details_ready,
            egui::Button::new(icons::labeled(icons::MIXIMAGE, "Generate Miximages")),
        )
        .clicked()
    {
        backend::assets::regenerate_miximages_for_selection(app, console_idx, ctx);
        ui.close();
    }

    if !app.ui_state.selected_entries.is_empty() {
        ui.separator();
        if ui
            .add_enabled(
                details_ready,
                egui::Button::new(icons::labeled(icons::RESCAN, "Rescan")),
            )
            .clicked()
        {
            backend::scan::rescan_selected_entries(app, console_idx, ctx);
            ui.close();
        }
        if ui
            .add_enabled(
                details_ready,
                egui::Button::new(icons::labeled(icons::HASH, "Calculate Missing Hashes")),
            )
            .clicked()
        {
            backend::hash::compute_hashes_for_selection(app, console_idx);
            ui.close();
        }
        if ui
            .add_enabled(
                details_ready,
                egui::Button::new(icons::labeled(icons::HASH, "Recalculate Hashes")),
            )
            .clicked()
        {
            backend::hash::recompute_hashes_for_selection(app, console_idx);
            ui.close();
        }
    }

    let actions = selected_releases
        .iter()
        .filter_map(|release| release.action.clone())
        .collect::<Vec<_>>();
    if !selected_releases.is_empty() {
        ui.separator();
        let ready = actions.len() == selected_releases.len()
            && actions.iter().all(|action| {
                action.buildable && (!action.needs_playable || action.preferred_format.is_some())
            });
        let needs_playable = actions.iter().any(|action| action.needs_playable);
        let needs_verification = actions.iter().any(|action| {
            action
                .carriers
                .iter()
                .any(|carrier| !carrier.catalog_verified)
        });
        let (icon, label) = if !needs_playable {
            (icons::VERIFY, "Verify Selected Archives")
        } else if needs_verification {
            (icons::VERIFY, "Verify & Make Selected Playable")
        } else {
            (icons::BUILD_PLAYABLE, "Make Selected Playable")
        };
        if ui
            .add_enabled(ready, egui::Button::new(icons::labeled(icon, label)))
            .on_disabled_hover_text(
                "Every selected archive must be complete and have a preferred playable format",
            )
            .clicked()
        {
            let folder_name = app.browser.consoles[console_idx].folder_name.clone();
            for action in actions {
                let format = action
                    .preferred_format
                    .as_deref()
                    .and_then(parse_archive_playable_format)
                    .or_else(|| {
                        (!action.needs_playable)
                            .then_some(retro_junk_archive::RepresentationFormat::Rom)
                    });
                if let Some(format) = format {
                    backend::playable_build::start(app, action, &format, folder_name.clone(), ctx);
                }
            }
            ui.close();
        }
    }
}

fn parse_archive_playable_format(value: &str) -> Option<retro_junk_archive::RepresentationFormat> {
    match value {
        "rom" => Some(retro_junk_archive::RepresentationFormat::Rom),
        "chd" => Some(retro_junk_archive::RepresentationFormat::Chd),
        "rvz" => Some(retro_junk_archive::RepresentationFormat::Rvz),
        "iso" => Some(retro_junk_archive::RepresentationFormat::Iso),
        "cue_bin" | "cue-bin" => Some(retro_junk_archive::RepresentationFormat::CueBin),
        _ => None,
    }
}

/// Render the "Set Region" submenu with recommended and other regions.
fn show_set_region_submenu(ui: &mut egui::Ui, app: &mut RetroJunkApp, console_idx: usize) {
    // Pre-compute the intersection of detected regions across selected entries.
    // Entries with no detected regions are excluded from the intersection so they
    // don't narrow the recommendations to empty.
    let console = &app.browser.consoles[console_idx];
    let mut recommended: Option<HashSet<Region>> = None;
    for &i in &app.ui_state.selected_entries {
        if let Some(entry) = console.entry_by_id(i)
            && let Some(ref id) = entry.identification
            && !id.regions.is_empty()
        {
            let set: HashSet<Region> = id.regions.iter().copied().collect();
            recommended = Some(match recommended {
                Some(acc) => acc.intersection(&set).copied().collect(),
                None => set,
            });
        }
    }
    let recommended = recommended.unwrap_or_default();

    ui.menu_button(icons::labeled(icons::REGION, "Set Region"), |ui| {
        if ui.button("Auto-detect").clicked() {
            let ids: Vec<_> = app.ui_state.selected_entries.iter().copied().collect();
            app.set_entry_regions(ids, None, ui.ctx());
            ui.close();
        }

        if !recommended.is_empty() {
            ui.separator();
            ui.label("Recommended");
            for &region in Region::ALL {
                if recommended.contains(&region) && ui.button(region.name()).clicked() {
                    let ids: Vec<_> = app.ui_state.selected_entries.iter().copied().collect();
                    app.set_entry_regions(ids, Some(region), ui.ctx());
                    ui.close();
                }
            }
        }

        ui.separator();
        ui.label("Other Regions");
        for &region in Region::ALL {
            if !recommended.contains(&region) && ui.button(region.name()).clicked() {
                let ids: Vec<_> = app.ui_state.selected_entries.iter().copied().collect();
                app.set_entry_regions(ids, Some(region), ui.ctx());
                ui.close();
            }
        }
    });
}

/// Collect a field from all selected entries, joining with newlines.
/// Entries where the extractor returns `None` are skipped.
fn collect_selected_field(
    app: &RetroJunkApp,
    console_idx: usize,
    extractor: impl Fn(&crate::state::LibraryEntry) -> Option<String>,
) -> String {
    let console = &app.browser.consoles[console_idx];
    let mut values: Vec<String> = app
        .ui_state
        .selected_entries
        .iter()
        .copied()
        .filter_map(|i| {
            let entry = console.entry_by_id(i)?;
            extractor(entry)
        })
        .collect();
    values.sort();
    values.join("\n")
}

/// Serial for a table row: the entry's own, falling back to the first
#[derive(Clone, Copy, Default)]
struct EntryListColumns {
    internal_name: bool,
    region: bool,
    dat_match: bool,
}

fn projected_regions(projection: &retro_junk_db::LibraryEntryListItem) -> String {
    if !projection.region_override.is_empty() {
        return format!("{}*", region_code(&projection.region_override));
    }
    projection
        .detected_regions
        .iter()
        .map(|region| region_code(region))
        .collect::<Vec<_>>()
        .join(", ")
}

fn region_code(region: &str) -> String {
    match region
        .to_ascii_lowercase()
        .replace([' ', '-'], "_")
        .as_str()
    {
        "japan" => "JPN".to_owned(),
        "usa" | "united_states" => "USA".to_owned(),
        "europe" => "EUR".to_owned(),
        "australia" => "AUS".to_owned(),
        "korea" => "KOR".to_owned(),
        "china" => "CHN".to_owned(),
        "taiwan" => "TWN".to_owned(),
        "asia" => "ASI".to_owned(),
        "brazil" => "BRA".to_owned(),
        "latinamerica" | "latin_america" => "LAT".to_owned(),
        "world" => "WLD".to_owned(),
        "unknown" => "UNK".to_owned(),
        _ => region.to_owned(),
    }
}

#[derive(Clone)]
struct RowData {
    entry_id: Option<retro_junk_db::LibraryEntryId>,
    /// Ordinary playable rows grouped under an archival release.
    bound_entry_ids: Vec<retro_junk_db::LibraryEntryId>,
    archive_release_id: Option<String>,
    status: EntryStatus,
    has_broken_refs: bool,
    has_hash_warnings: bool,
    has_cue_compat_issues: bool,
    asset_status: AssetStatus,
    name: String,
    file_path: Option<PathBuf>,
    internal_name: String,
    regions: String,
    dat_match: String,
    availability: AvailabilityState,
}

#[derive(Clone)]
enum AvailabilityState {
    ArchivedOnly,
    IncompleteArchive,
    PlayableOnly { format: String },
    ArchivedAndPlayable { format: String },
    IncompleteArchiveAndPlayable { format: String },
    PreferredFormatMismatch { actual: String, preferred: String },
}

impl AvailabilityState {
    fn from_archive(release: &retro_junk_db::ArchivedLibraryListItem) -> Self {
        let summary = &release.summary;
        let playable =
            summary.playable_present_count > 0 || !release.playable_library_entries.is_empty();
        let actual = archive_playable_formats(release);
        if summary.archive_complete {
            if release
                .action
                .as_ref()
                .is_some_and(|action| action.needs_playable)
                && playable
            {
                return Self::PreferredFormatMismatch {
                    actual,
                    preferred: release
                        .action
                        .as_ref()
                        .and_then(|action| action.preferred_format.as_deref())
                        .map_or_else(|| "the configured format".to_owned(), display_format),
                };
            }
            if playable {
                Self::ArchivedAndPlayable { format: actual }
            } else {
                Self::ArchivedOnly
            }
        } else if playable {
            Self::IncompleteArchiveAndPlayable { format: actual }
        } else {
            Self::IncompleteArchive
        }
    }

    fn from_projection(projection: &retro_junk_db::LibraryEntryListItem) -> Self {
        let format = display_format(&projection.playable_format);
        if !projection.archived {
            return Self::PlayableOnly { format };
        }
        if !projection.archive_complete {
            return Self::IncompleteArchiveAndPlayable { format };
        }
        if let Some(preferred) = projection.preferred_format.as_deref()
            && !formats_satisfy_policy(&projection.playable_format, preferred)
        {
            return Self::PreferredFormatMismatch {
                actual: format,
                preferred: display_format(preferred),
            };
        }
        Self::ArchivedAndPlayable { format }
    }

    fn label(&self) -> &str {
        match self {
            Self::ArchivedOnly => "Archived only",
            Self::IncompleteArchive => "Archive incomplete",
            Self::PlayableOnly { .. } => "Playable only",
            Self::ArchivedAndPlayable { .. } => "Archived + playable",
            Self::IncompleteArchiveAndPlayable { .. } => "Archive incomplete + playable",
            Self::PreferredFormatMismatch { .. } => "Wrong playable format",
        }
    }

    fn tooltip(&self) -> String {
        match self {
            Self::ArchivedOnly => {
                "The complete, verified release is archived but no playable copy is present"
                    .to_owned()
            }
            Self::IncompleteArchive => {
                "The archive does not yet contain every expected, catalog-verified disc".to_owned()
            }
            Self::PlayableOnly { format } => {
                format!("Playable as {format}, but no matching archival carrier is indexed")
            }
            Self::ArchivedAndPlayable { format } => {
                format!("Archived and playable as {format}")
            }
            Self::IncompleteArchiveAndPlayable { format } => {
                format!(
                    "Playable as {format}; the archive does not yet have every expected disc catalog-verified"
                )
            }
            Self::PreferredFormatMismatch { actual, preferred } => {
                format!("Playable as {actual}; the archive policy prefers {preferred}")
            }
        }
    }
}

fn archive_playable_formats(release: &retro_junk_db::ArchivedLibraryListItem) -> String {
    let mut formats = release
        .playable_representations
        .iter()
        .map(|representation| display_format(&representation.format))
        .chain(
            release
                .playable_library_entries
                .iter()
                .map(|entry| display_format(&entry.playable_format)),
        )
        .collect::<Vec<_>>();
    formats.sort();
    formats.dedup();
    if formats.is_empty() {
        "unknown format".to_owned()
    } else {
        formats.join(" + ")
    }
}

fn aggregate_asset_status(statuses: impl Iterator<Item = AssetStatus>) -> AssetStatus {
    let mut saw_status = false;
    let mut best = AssetStatus::Unknown;
    for status in statuses {
        saw_status = true;
        best = match (best, status) {
            (AssetStatus::Complete, _) | (_, AssetStatus::Complete) => AssetStatus::Complete,
            (
                AssetStatus::Partial {
                    found: left,
                    total: left_total,
                },
                AssetStatus::Partial {
                    found: right,
                    total: right_total,
                },
            ) => AssetStatus::Partial {
                found: left.max(right),
                total: left_total.max(right_total),
            },
            (AssetStatus::Partial { found, total }, _)
            | (_, AssetStatus::Partial { found, total }) => AssetStatus::Partial { found, total },
            (AssetStatus::None, _) | (_, AssetStatus::None) => AssetStatus::None,
            _ => AssetStatus::Unknown,
        };
    }
    saw_status.then_some(best).unwrap_or(AssetStatus::Unknown)
}

fn archive_asset_status(assets: &[retro_junk_db::ArchivedReleaseAsset]) -> AssetStatus {
    let asset_count = assets
        .iter()
        .map(|asset| asset.asset_type.as_str())
        .collect::<HashSet<_>>()
        .len();
    if asset_count == 0 {
        AssetStatus::None
    } else {
        let total = u8::try_from(crate::state::SCRAPEABLE_ASSET_TYPES.len()).unwrap_or(u8::MAX);
        let found = u8::try_from(asset_count).unwrap_or(u8::MAX).min(total);
        if found == total {
            AssetStatus::Complete
        } else {
            AssetStatus::Partial { found, total }
        }
    }
}

fn select_archive_release(
    app: &mut RetroJunkApp,
    release_id: &str,
    bound_entry_ids: &[retro_junk_db::LibraryEntryId],
    modifiers: egui::Modifiers,
    ctx: &egui::Context,
) {
    if modifiers.ctrl || modifiers.command {
        if app.ui_state.selected_archive_releases.remove(release_id) {
            for entry_id in bound_entry_ids {
                app.ui_state.selected_entries.remove(entry_id);
            }
            app.ui_state.focused_archive_release = app
                .ui_state
                .selected_archive_releases
                .iter()
                .next()
                .cloned();
            app.ui_state.focused_entry = app.ui_state.selected_entries.iter().next().copied();
            return;
        }
        app.ui_state
            .selected_archive_releases
            .insert(release_id.to_owned());
        app.ui_state
            .selected_entries
            .extend(bound_entry_ids.iter().copied());
    } else {
        app.ui_state.selected_entries = bound_entry_ids.iter().copied().collect();
        app.ui_state.selected_archive_releases.clear();
        app.ui_state
            .selected_archive_releases
            .insert(release_id.to_owned());
    }
    app.ui_state.focused_entry = None;
    app.ui_state.focused_archive_release = Some(release_id.to_owned());
    app.request_selected_entry_details(ctx);
}

fn row_is_focused(
    entry_id: Option<retro_junk_db::LibraryEntryId>,
    archive_release_id: Option<&str>,
    focused_entry: Option<retro_junk_db::LibraryEntryId>,
    focused_archive_release: Option<&str>,
) -> bool {
    entry_id.is_some_and(|id| focused_entry == Some(id))
        || archive_release_id.is_some_and(|release_id| focused_archive_release == Some(release_id))
}

fn row_supports_context_menu(
    entry_id: Option<retro_junk_db::LibraryEntryId>,
    archive_release_id: Option<&str>,
) -> bool {
    entry_id.is_some() || archive_release_id.is_some()
}

fn formats_satisfy_policy(actual: &str, preferred: &str) -> bool {
    let actual = actual.to_ascii_lowercase().replace('-', "_");
    let preferred = preferred.to_ascii_lowercase().replace('-', "_");
    if actual == preferred {
        return true;
    }
    match preferred.as_str() {
        "cue_bin" => matches!(actual.as_str(), "cue" | "bin"),
        "rom" => !matches!(
            actual.as_str(),
            "chd" | "rvz" | "iso" | "cue" | "bin" | "gdi" | "cso" | "dax"
        ),
        _ => false,
    }
}

fn display_format(format: &str) -> String {
    if format.is_empty() {
        "unknown format".to_owned()
    } else {
        format.to_ascii_uppercase().replace('_', "-")
    }
}

fn projection_status(status: &str, tag: &str) -> EntryStatus {
    match tag {
        "homebrew" => EntryStatus::Tagged(retro_junk_catalog::CatalogTag::Homebrew),
        "modded" => EntryStatus::Tagged(retro_junk_catalog::CatalogTag::Modded),
        _ => match status {
            "matched" => EntryStatus::Matched,
            "likely" => EntryStatus::LikelyMatched,
            "ambiguous" => EntryStatus::Ambiguous,
            "unrecognized" => EntryStatus::Unrecognized,
            _ => EntryStatus::Unknown,
        },
    }
}

fn handle_row_click(
    app: &mut RetroJunkApp,
    entry_id: retro_junk_db::LibraryEntryId,
    modifiers: egui::Modifiers,
    visible_ids: &[retro_junk_db::LibraryEntryId],
) {
    if modifiers.ctrl || modifiers.command {
        // Toggle selection
        if app.ui_state.selected_entries.contains(&entry_id) {
            app.ui_state.selected_entries.remove(&entry_id);
            app.ui_state.focused_entry = app.ui_state.selected_entries.iter().next().copied();
            return;
        }
        app.ui_state.selected_entries.insert(entry_id);
    } else if modifiers.shift {
        // Range select
        if let Some(focused) = app.ui_state.focused_entry
            && let (Some(start), Some(end)) = (
                visible_ids.iter().position(|id| *id == focused),
                visible_ids.iter().position(|id| *id == entry_id),
            )
        {
            for &id in &visible_ids[start.min(end)..=start.max(end)] {
                app.ui_state.selected_entries.insert(id);
            }
        } else {
            app.ui_state.selected_entries.clear();
            app.ui_state.selected_archive_releases.clear();
            app.ui_state.selected_entries.insert(entry_id);
        }
    } else {
        // Single select
        app.ui_state.selected_entries.clear();
        app.ui_state.selected_archive_releases.clear();
        app.ui_state.selected_entries.insert(entry_id);
    }

    app.ui_state.focused_entry = Some(entry_id);
}

/// Paint text in a table cell without creating a widget.
///
/// This avoids registering a `WidgetRect` that would intercept pointer events,
/// allowing the cell's own response (set via `TableBuilder::sense`) to handle
/// all click and context-menu interaction.
///
/// Text is clipped to the cell's available rect so it doesn't bleed into
/// adjacent columns.
/// Render tag-related context menu items for a single entry.
fn show_tag_menu_items(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    console_idx: usize,
    entry_id: retro_junk_db::LibraryEntryId,
) {
    let Some(entry_idx) = app.browser.consoles[console_idx].entry_index(entry_id) else {
        return;
    };
    let Some(entry) = app
        .browser
        .consoles
        .get(console_idx)
        .and_then(|c| c.entries.get(entry_idx))
    else {
        return;
    };
    let effective = entry.effective_status();
    let platform_id = app
        .context
        .get_by_platform(app.browser.consoles[console_idx].platform)
        .map_or_else(
            || app.browser.consoles[console_idx].folder_name.clone(),
            |registered| registered.analyzer.short_name().to_owned(),
        );
    let is_disc_based = app
        .context
        .get_by_platform(app.browser.consoles[console_idx].platform)
        .is_some_and(|registered| {
            registered.analyzer.dat_source() == retro_junk_lib::DatSource::Redump
        });
    let disc_number_required = mod_disc_number_required(is_disc_based, &entry.game_entry);

    match effective {
        EntryStatus::Unrecognized => {
            ui.separator();
            if ui
                .button(icons::labeled(icons::TAG, "Mark as Homebrew\u{2026}"))
                .clicked()
            {
                // Pre-fill with cleaned filename
                let name = entry.game_entry.display_name().to_string();
                app.ui_state.tag_dialog = TagDialog::Homebrew {
                    name,
                    console_id: app.browser.consoles[console_idx].id.unwrap(),
                    entry_id,
                };
                ui.close();
            }
            if ui
                .button(icons::labeled(
                    icons::TAG,
                    "Mark as Modded Version of\u{2026}",
                ))
                .clicked()
            {
                app.ui_state.tag_dialog = TagDialog::ModSearch {
                    query: String::new(),
                    results: Vec::new(),
                    selected: None,
                    platform_id,
                    disc_number_required,
                    disc_number: String::new(),
                    console_id: app.browser.consoles[console_idx].id.unwrap(),
                    entry_id,
                };
                ui.close();
            }
        }
        EntryStatus::Tagged(_) => {
            ui.separator();
            if ui
                .button(icons::labeled(icons::TAG, "Remove Tag"))
                .clicked()
            {
                app.set_entry_tags([entry_id], None, ui.ctx());
                ui.close();
            }
        }
        _ => {
            ui.separator();
            if ui
                .button(icons::labeled(
                    icons::TAG,
                    "Mark as Modded Version of\u{2026}",
                ))
                .clicked()
            {
                app.ui_state.tag_dialog = TagDialog::ModSearch {
                    query: String::new(),
                    results: Vec::new(),
                    selected: None,
                    platform_id,
                    disc_number_required,
                    disc_number: String::new(),
                    console_id: app.browser.consoles[console_idx].id.unwrap(),
                    entry_id,
                };
                ui.close();
            }
        }
    }
}

fn mod_disc_number_required(
    is_disc_based: bool,
    entry: &retro_junk_lib::scanner::GameEntry,
) -> bool {
    is_disc_based
        && matches!(
            entry,
            retro_junk_lib::scanner::GameEntry::SingleFile(path)
                if !path.extension().and_then(|extension| extension.to_str())
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("m3u"))
        )
}

fn paint_cell_text(ui: &mut egui::Ui, text: &str) {
    if text.is_empty() {
        return;
    }
    let rect = ui.max_rect();
    let font_id = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    ui.painter().with_clip_rect(rect).text(
        rect.left_center(),
        egui::Align2::LEFT_CENTER,
        text,
        font_id,
        color,
    );
}

#[cfg(test)]
#[path = "game_table_tests.rs"]
mod interaction_tests;

#[cfg(test)]
mod tests {
    use super::projection_status;
    use crate::state::EntryStatus;
    use retro_junk_catalog::CatalogTag;
    use retro_junk_lib::scanner::GameEntry;

    #[test]
    fn projected_tags_override_stored_analysis_status() {
        assert_eq!(
            projection_status("unrecognized", "homebrew"),
            EntryStatus::Tagged(CatalogTag::Homebrew)
        );
        assert_eq!(
            projection_status("matched", "modded"),
            EntryStatus::Tagged(CatalogTag::Modded)
        );
        assert_eq!(projection_status("ambiguous", ""), EntryStatus::Ambiguous);
        assert_eq!(projection_status("likely", ""), EntryStatus::LikelyMatched);
    }

    #[test]
    fn loose_disc_requires_number_but_m3u_game_folder_does_not() {
        assert!(super::mod_disc_number_required(
            true,
            &GameEntry::SingleFile("/roms/saturn/Modded Game.chd".into())
        ));
        assert!(!super::mod_disc_number_required(
            true,
            &GameEntry::MultiDisc {
                name: "Modded Game.m3u".to_owned(),
                files: vec!["/roms/psx/Modded Game.m3u/disc-1.chd".into()],
            }
        ));
        assert!(!super::mod_disc_number_required(
            true,
            &GameEntry::SingleFile("/roms/psx/Modded Game.m3u".into())
        ));
        assert!(!super::mod_disc_number_required(
            false,
            &GameEntry::SingleFile("/roms/nes/Modded Game.nes".into())
        ));
    }

    #[test]
    fn native_rom_extensions_satisfy_rom_policy() {
        assert!(super::formats_satisfy_policy("nes", "rom"));
        assert!(super::formats_satisfy_policy("sfc", "rom"));
        assert!(!super::formats_satisfy_policy("chd", "rom"));
        assert!(super::formats_satisfy_policy("cue", "cue_bin"));
    }

    #[test]
    fn absent_archive_identity_never_focuses_an_ordinary_row() {
        let entry = retro_junk_db::LibraryEntryId(7);
        assert!(!super::row_is_focused(Some(entry), None, None, None));
        assert!(super::row_is_focused(Some(entry), None, Some(entry), None));
        assert!(super::row_is_focused(
            None,
            Some("release"),
            None,
            Some("release")
        ));
    }

    #[test]
    fn archive_only_rows_support_context_menus() {
        assert!(super::row_supports_context_menu(None, Some("release")));
        assert!(super::row_supports_context_menu(
            Some(retro_junk_db::LibraryEntryId(1)),
            None
        ));
        assert!(!super::row_supports_context_menu(None, None));
    }

    #[test]
    fn control_click_toggles_entries_without_losing_the_other_selection() {
        let mut app = crate::app::RetroJunkApp::with_parts(
            &egui::Context::default(),
            crate::settings::AppSettings::default(),
            None,
            None,
        );
        let first = retro_junk_db::LibraryEntryId(1);
        let second = retro_junk_db::LibraryEntryId(2);
        let visible = [first, second];

        super::handle_row_click(&mut app, first, egui::Modifiers::NONE, &visible);
        super::handle_row_click(
            &mut app,
            second,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
            &visible,
        );
        assert_eq!(app.ui_state.selected_entries.len(), 2);

        super::handle_row_click(
            &mut app,
            first,
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
            &visible,
        );
        assert_eq!(
            app.ui_state.selected_entries,
            std::collections::HashSet::from([second])
        );
        assert_eq!(app.ui_state.focused_entry, Some(second));
    }

    #[test]
    fn control_click_toggles_logical_archive_rows() {
        let ctx = egui::Context::default();
        let mut app = crate::app::RetroJunkApp::with_parts(
            &ctx,
            crate::settings::AppSettings::default(),
            None,
            None,
        );

        super::select_archive_release(&mut app, "release-a", &[], egui::Modifiers::NONE, &ctx);
        super::select_archive_release(
            &mut app,
            "release-b",
            &[],
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
            &ctx,
        );
        assert_eq!(app.ui_state.selected_archive_releases.len(), 2);

        super::select_archive_release(
            &mut app,
            "release-a",
            &[],
            egui::Modifiers {
                ctrl: true,
                ..Default::default()
            },
            &ctx,
        );
        assert_eq!(
            app.ui_state.selected_archive_releases,
            std::collections::HashSet::from(["release-b".to_owned()])
        );
    }
}
