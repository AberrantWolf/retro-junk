//! Review inbox: everything waiting on a decision.
//!
//! Automation is allowed to act on what it is confident about; everything
//! else lands here rather than happening silently or being lost in a log.
//! Three sources, one list: open suggestions (proposed-but-unapplied
//! commands), incoming packages whose pre-processing ended in an honest
//! error state, and a pointer to unresolved catalog disagreements.
//!
//! The shape of this view is set by how big that list actually gets. A card
//! per row is right for five rows and unusable at a thousand, where the piles
//! are what matter — every `.txt` in the library, every `GameCube` dump the
//! archive cannot accept yet — and where the useful action is on a whole pile
//! at once. So: one compact line per row, folded into groups, described by a
//! path pattern rather than selected by hand, and acted on in bulk.
//!
//! **Only the visible rows are laid out.** Everything on screen is flattened
//! into a list of one-line [`VisualRow`]s and drawn through
//! `ScrollArea::show_rows`, which asks for a row count and hands back just the
//! range that fits the viewport. A thousand-row backlog costs the same per
//! frame as a ten-row one. Expanding a row splices its detail in as more
//! one-line rows rather than making one row taller, which is what keeps every
//! row the same height and the whole list virtualized.

use crate::app::RetroJunkApp;
use crate::backend::inbox::{InboxItem, InboxSort};
use crate::state::{InboxChoice, InboxConfirm, InboxConfirmKind, InboxIgnoreDraft};
use crate::widgets::icons;

/// Height of one line in the list. Uniform by construction — see the module
/// note on why.
const ROW_HEIGHT: f32 = 22.0;

/// What the user clicked, applied after the list finishes rendering so the
/// row loop keeps its borrow of the loaded contents.
enum Action {
    Apply { id: i64, label: String },
    Choose(InboxChoice),
    Dismiss(Vec<i64>),
    Reveal(std::path::PathBuf),
    ToggleGroup(String),
    ToggleRow(i64),
    Focus(i64),
    IgnoreOne(String),
    Retry(retro_junk_db::work::IncomingPackage),
    ForgetPackage(String),
}

/// One line on screen.
enum VisualRow<'a> {
    /// A fold-away pile header with its count.
    Group { name: &'a str, count: usize },
    /// A review, one line.
    Item(&'a InboxItem),
    /// One line of an expanded review's detail.
    Detail { id: i64, text: &'a str },
    /// The buttons belonging to an expanded review.
    Actions(&'a InboxItem),
    /// A failed incoming package.
    Package(&'a retro_junk_db::work::IncomingPackage),
    /// A plain heading between sections.
    Heading(String),
}

pub fn show(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    // Consume the dirty flag only when a load can actually start: `load`
    // returns without doing anything while one is in flight, and a change
    // committed after that query ran would otherwise have its signal eaten —
    // resolved rows would keep showing until a manual refresh. Left set, the
    // flag re-fires on the frame after the in-flight load completes.
    if !app.ui_state.inbox_loading && std::mem::take(&mut app.ui_state.inbox_dirty) {
        crate::backend::inbox::load(app, ctx);
    }

    header(ui, app, ctx);
    ui.separator();

    // Built once per frame from the loaded rows; the borrow ends before any
    // action runs.
    let filter = retro_junk_backend::suggestions::SuggestionFilter::new(
        app.ui_state.inbox_ui.filter_kind.as_deref(),
        &app.ui_state.inbox_ui.filter_text,
    );
    let mut action = None;
    let scroll_to = std::mem::take(&mut app.ui_state.inbox_ui.scroll_to_cursor);
    let cursor = app.ui_state.inbox_ui.cursor;
    {
        let inbox = &app.ui_state.inbox;
        let ui_state = &app.ui_state.inbox_ui;
        let visible = inbox.visible(&filter, ui_state.sort);
        let rows = flatten(&visible, inbox, ui_state);

        if rows.is_empty() {
            empty_state(ui, inbox, &filter);
        } else {
            let mut scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
            // Every row is the same height, so putting the keyboard cursor on
            // screen is arithmetic rather than a search: its offset is its
            // position in the list. Scrolling to a row egui has not laid out —
            // which is the whole point of virtualizing — could not work any
            // other way.
            if scroll_to
                && let Some(index) = rows.iter().position(|visual| {
                    matches!(visual, VisualRow::Item(item)
                        if Some(item.suggestion.id) == cursor)
                })
            {
                let target = index as f32 * ROW_HEIGHT;
                let viewport = ui.available_height();
                let current = ui
                    .ctx()
                    .data_mut(|data| *data.get_temp_mut_or(egui::Id::new("inbox-scroll"), 0.0_f32));
                // Only move when the row is off screen, so holding a key
                // scrolls one line at a time instead of recentring.
                let offset = if target < current {
                    target
                } else if target + ROW_HEIGHT > current + viewport {
                    target + ROW_HEIGHT - viewport
                } else {
                    current
                };
                scroll = scroll.vertical_scroll_offset(offset.max(0.0));
            }
            let output = scroll.show_rows(ui, ROW_HEIGHT, rows.len(), |ui, range| {
                for index in range {
                    if let Some(chosen) = row(ui, &rows[index], app) {
                        action = Some(chosen);
                    }
                }
            });
            ui.ctx().data_mut(|data| {
                data.insert_temp(egui::Id::new("inbox-scroll"), output.state.offset.y);
            });
        }
    }

    dialogs(ui, app, ctx);
    keyboard(ui, app, &filter, ctx);

    if let Some(chosen) = action {
        run(app, chosen, ctx);
    }
}

/// The heading, the filter, the sort, and the bulk buttons.
fn header(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    let filter = retro_junk_backend::suggestions::SuggestionFilter::new(
        app.ui_state.inbox_ui.filter_kind.as_deref(),
        &app.ui_state.inbox_ui.filter_text,
    );
    let total = app.ui_state.inbox.items.len();
    let shown: Vec<i64> = app
        .ui_state
        .inbox
        .items
        .iter()
        .filter(|item| filter.matches(&item.suggestion))
        .map(|item| item.suggestion.id)
        .collect();
    let appliable: Vec<i64> = app
        .ui_state
        .inbox
        .items
        .iter()
        .filter(|item| {
            filter.matches(&item.suggestion)
                && item.actions.applicable
                && item.actions.choices.is_empty()
        })
        .map(|item| item.suggestion.id)
        .collect();
    let filtering = shown.len() != total;

    ui.horizontal(|ui| {
        ui.heading("Inbox");
        if app.ui_state.inbox_loading {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button(icons::labeled(icons::RESCAN, "Refresh"))
                .clicked()
            {
                crate::backend::inbox::load(app, ctx);
            }
            if filtering {
                ui.weak(format!("showing {} of {total}", shown.len()));
            } else if total > 0 {
                ui.weak(format!("{total} waiting"));
            }
        });
    });

    ui.horizontal_wrapped(|ui| {
        ui.label(icons::FILTER);
        let response = ui.add(
            egui::TextEdit::singleline(&mut app.ui_state.inbox_ui.filter_text)
                .desired_width(240.0)
                .hint_text("*.txt, */rvz/*, or any part of a path"),
        );
        response.on_hover_text(
            "Describe a group of files: `*` matches anything including slashes, \
             `?` one character, `[0-9]` a set. A bare word matches anywhere in the path.",
        );
        if !app.ui_state.inbox_ui.filter_text.is_empty() && ui.small_button("✕").clicked() {
            app.ui_state.inbox_ui.filter_text.clear();
        }

        // Kind chips, each showing how much of the backlog it is.
        let counts = app.ui_state.inbox.counts_by_kind.clone();
        let selected_kind = app.ui_state.inbox_ui.filter_kind.clone();
        if ui
            .selectable_label(selected_kind.is_none(), format!("All {total}"))
            .clicked()
        {
            app.ui_state.inbox_ui.filter_kind = None;
        }
        for (kind, count) in counts {
            let selected = selected_kind.as_deref() == Some(kind.as_str());
            if ui
                .selectable_label(selected, format!("{} {count}", kind_label(&kind)))
                .clicked()
            {
                app.ui_state.inbox_ui.filter_kind = if selected { None } else { Some(kind) };
            }
        }

        egui::ComboBox::from_id_salt("inbox-sort")
            .selected_text(app.ui_state.inbox_ui.sort.label())
            .show_ui(ui, |ui| {
                for option in InboxSort::ALL {
                    ui.selectable_value(&mut app.ui_state.inbox_ui.sort, option, option.label());
                }
            });
    });

    // Bulk actions act on the filtered set, never on a hand-made selection —
    // the count in the label is the same number the header just showed.
    if !shown.is_empty() {
        ui.horizontal_wrapped(|ui| {
            let dismiss = ui
                .button(format!("Dismiss {} shown", shown.len()))
                .on_hover_text(
                    "Closes these review rows. It never touches a file, and the rows can be \
                     put back.",
                );
            if dismiss.clicked() {
                app.ui_state.inbox_ui.confirm = Some(InboxConfirm {
                    kind: InboxConfirmKind::Dismiss,
                    ids: shown.clone(),
                    description: describe_filter(app),
                });
            }
            if !appliable.is_empty()
                && ui
                    .button(icons::labeled(
                        icons::VERIFY,
                        &format!("Apply {} shown", appliable.len()),
                    ))
                    .on_hover_text(
                        "Runs each of these through the same path Apply uses, one at a time",
                    )
                    .clicked()
            {
                app.ui_state.inbox_ui.confirm = Some(InboxConfirm {
                    kind: InboxConfirmKind::Apply,
                    ids: appliable.clone(),
                    description: describe_filter(app),
                });
            }
            // Offered whenever a pattern has been typed, including one that
            // happens to match everything currently open — a library where
            // every stray really is a `.txt` is exactly the case worth one
            // rule, and the dialog states the count before anything is saved.
            if !app.ui_state.inbox_ui.filter_text.trim().is_empty()
                && ui
                    .button("Never ask again…")
                    .on_hover_text(
                        "Records a durable rule so the next sweep does not file these at all",
                    )
                    .clicked()
            {
                app.ui_state.inbox_ui.ignore_draft = Some(InboxIgnoreDraft {
                    pattern: app.ui_state.inbox_ui.filter_text.trim().to_owned(),
                    note: String::new(),
                });
            }
            let rule_count = app.ui_state.inbox.ignore_rules.len();
            if rule_count > 0
                && ui
                    .selectable_label(
                        app.ui_state.inbox_ui.show_ignore_rules,
                        format!("{rule_count} ignore rule(s)"),
                    )
                    .clicked()
            {
                app.ui_state.inbox_ui.show_ignore_rules = !app.ui_state.inbox_ui.show_ignore_rules;
            }
        });
    }

    // The undo banner outlives one frame on purpose: a bulk dismissal is the
    // one action here that closes hundreds of rows at once, and the offer to
    // put them back should still be there after the list redraws.
    if let Some(undo) = &app.ui_state.inbox_ui.undo {
        let ids = undo.ids.clone();
        let label = undo.label.clone();
        ui.horizontal(|ui| {
            ui.colored_label(crate::theme::STATUS_INFO, label);
            if ui.button("Undo").clicked() {
                crate::backend::inbox::reopen(app, ids, ctx);
                app.ui_state.inbox_ui.undo = None;
            }
            if ui.small_button("✕").clicked() {
                app.ui_state.inbox_ui.undo = None;
            }
        });
    }

    if app.ui_state.inbox_ui.show_ignore_rules {
        ignore_rule_list(ui, app, ctx);
    }
}

/// The rules in force, and a way out of each one.
fn ignore_rule_list(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    let rules = app.ui_state.inbox.ignore_rules.clone();
    let mut revoke = None;
    egui::Frame::group(ui.style())
        .fill(ui.visuals().faint_bg_color)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.weak("Files matching these are never filed for review. Revoking one makes the next sweep file them again.");
            for rule in &rules {
                ui.horizontal(|ui| {
                    ui.monospace(&rule.pattern);
                    if !rule.note.is_empty() {
                        ui.weak(&rule.note);
                    }
                    if ui.small_button("Revoke").clicked() {
                        revoke = Some(rule.pattern.clone());
                    }
                });
            }
        });
    if let Some(pattern) = revoke {
        crate::backend::inbox::unignore(app, pattern, ctx);
    }
}

/// Flatten groups, rows, and any expanded detail into one list of lines.
fn flatten<'a>(
    visible: &[&'a InboxItem],
    inbox: &'a crate::backend::inbox::InboxContents,
    ui_state: &crate::state::InboxUiState,
) -> Vec<VisualRow<'a>> {
    let mut rows = Vec::new();
    let mut current_group: Option<&str> = None;
    let mut collapsed_group = false;

    // Rows arrive already sorted; group headers are emitted as the group
    // changes, so a sort that interleaves groups simply produces more headers
    // rather than a wrong count.
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for item in visible {
        *counts.entry(item.group.as_str()).or_default() += 1;
    }

    for item in visible {
        if current_group != Some(item.group.as_str()) {
            current_group = Some(item.group.as_str());
            collapsed_group = ui_state.collapsed.contains(&item.group);
            rows.push(VisualRow::Group {
                name: &item.group,
                count: counts.get(item.group.as_str()).copied().unwrap_or(0),
            });
        }
        if collapsed_group {
            continue;
        }
        rows.push(VisualRow::Item(item));
        if ui_state.expanded.contains(&item.suggestion.id) {
            for detail in &item.details {
                rows.push(VisualRow::Detail {
                    id: item.suggestion.id,
                    text: detail,
                });
            }
            rows.push(VisualRow::Actions(item));
        }
    }

    if !inbox.failed_packages.is_empty() {
        rows.push(VisualRow::Heading(format!(
            "Incoming packages that could not be processed ({})",
            inbox.failed_packages.len()
        )));
        for package in &inbox.failed_packages {
            rows.push(VisualRow::Package(package));
        }
    }

    if inbox.unresolved_disagreements > 0 {
        rows.push(VisualRow::Heading(format!(
            "{} unresolved catalog disagreement(s) — review them under Tools → Browse → Disagreements",
            inbox.unresolved_disagreements
        )));
    }
    rows
}

/// Draw one line.
fn row(ui: &mut egui::Ui, visual: &VisualRow<'_>, app: &RetroJunkApp) -> Option<Action> {
    let mut action = None;
    match visual {
        VisualRow::Group { name, count } => {
            let collapsed = app.ui_state.inbox_ui.collapsed.contains(*name);
            ui.horizontal(|ui| {
                let arrow = if collapsed { "▸" } else { "▾" };
                if ui
                    .add(
                        egui::Button::new(
                            egui::RichText::new(format!("{arrow}  {name}  ·  {count}")).strong(),
                        )
                        .frame(false),
                    )
                    .clicked()
                {
                    action = Some(Action::ToggleGroup((*name).to_owned()));
                }
            });
        }
        VisualRow::Item(item) => {
            action = item_row(ui, item, app);
        }
        VisualRow::Detail { id, text } => {
            ui.horizontal(|ui| {
                ui.add_space(28.0);
                ui.weak(*text);
            });
            let _ = id;
        }
        VisualRow::Actions(item) => {
            action = action_row(ui, item, app.ui_state.inbox.playable_root.as_deref());
        }
        VisualRow::Package(package) => {
            action = package_row(ui, package);
        }
        VisualRow::Heading(text) => {
            ui.horizontal(|ui| {
                ui.add_space(4.0);
                ui.colored_label(crate::theme::STATUS_WARN, text.as_str());
            });
        }
    }
    action
}

/// One review, on one line.
fn item_row(ui: &mut egui::Ui, item: &InboxItem, app: &RetroJunkApp) -> Option<Action> {
    let mut action = None;
    let id = item.suggestion.id;
    let expanded = app.ui_state.inbox_ui.expanded.contains(&id);
    let focused = app.ui_state.inbox_ui.cursor == Some(id);
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        let arrow = if expanded { "▾" } else { "▸" };
        if ui
            .add(egui::Button::new(arrow).frame(false).small())
            .clicked()
        {
            action = Some(Action::ToggleRow(id));
        }
        let name = if focused {
            egui::RichText::new(&item.headline).strong().underline()
        } else {
            egui::RichText::new(&item.headline).strong()
        };
        // A missing file is the single most useful thing to see at a glance:
        // it means the review is about something that is no longer there.
        let name = if item.exists {
            name
        } else {
            name.color(crate::theme::STATUS_ERR)
        };
        if ui
            .add(egui::Label::new(name).sense(egui::Sense::click()))
            .clicked()
        {
            action = Some(Action::Focus(id));
        }
        if !item.location.is_empty() {
            ui.weak(&item.location);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.weak(confidence_label(item.suggestion.confidence));
            if !item.status.is_empty() {
                ui.label(&item.status);
            }
            if !item.actions.choices.is_empty() {
                ui.colored_label(
                    crate::theme::STATUS_WARN,
                    format!("{} candidates", item.actions.choices.len()),
                );
            }
        });
    });
    action
}

/// The buttons for an expanded review.
fn action_row(
    ui: &mut egui::Ui,
    item: &InboxItem,
    playable_root: Option<&std::path::Path>,
) -> Option<Action> {
    let mut action = None;
    let id = item.suggestion.id;
    ui.horizontal(|ui| {
        ui.add_space(28.0);
        if item.actions.choices.is_empty() {
            let apply = ui
                .add_enabled(
                    item.actions.applicable,
                    egui::Button::new(icons::labeled(icons::VERIFY, "Apply")),
                )
                .on_hover_text(
                    "Re-validates first, then runs the same command \
                     `retro-junk suggestions apply` runs",
                )
                .on_disabled_hover_text(
                    "Nothing here can be executed: the sweep found no candidate for this file, \
                     so it records a decision to make where the files are",
                );
            if apply.clicked() {
                action = Some(Action::Apply {
                    id,
                    label: item.headline.clone(),
                });
            }
        } else if ui
            .button(icons::labeled(icons::VERIFY, "Choose…"))
            .on_hover_text("This file matches several things; pick which one it is")
            .clicked()
        {
            action = Some(Action::Choose(InboxChoice {
                id,
                label: item.headline.clone(),
                candidates: item.actions.choices.clone(),
                selected: None,
            }));
        }
        if ui
            .button("Dismiss")
            .on_hover_text("Closes this row. The next sweep will file it again.")
            .clicked()
        {
            action = Some(Action::Dismiss(vec![id]));
        }
        if item.actions.ignorable
            && ui
                .button("Never ask again")
                .on_hover_text("Records a durable rule for this exact path")
                .clicked()
        {
            action = Some(Action::IgnoreOne(item.suggestion.target_id.clone()));
        }
        if item.exists
            && ui
                .button(icons::labeled(icons::REVEAL, crate::util::REVEAL_LABEL))
                .clicked()
            && let Some(path) = crate::backend::inbox::target_path(&item.suggestion, playable_root)
        {
            action = Some(Action::Reveal(path));
        }
        ui.weak(format!("seen {}", item.suggestion.created_at));
    });
    action
}

/// A failed incoming package, on one line.
fn package_row(
    ui: &mut egui::Ui,
    package: &retro_junk_db::work::IncomingPackage,
) -> Option<Action> {
    let mut action = None;
    ui.horizontal(|ui| {
        ui.add_space(12.0);
        ui.colored_label(crate::theme::STATUS_ERR, icons::WARNING);
        ui.strong(&package.path);
        if !package.detail.is_empty() {
            ui.weak(&package.detail);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .small_button("Forget")
                .on_hover_text(
                    "Stops tracking this package. The file is untouched, so a watcher that \
                     still sees it in the drop folder will pick it up again.",
                )
                .clicked()
            {
                action = Some(Action::ForgetPackage(package.path.clone()));
            }
            if ui
                .small_button(icons::labeled(icons::RESCAN, "Try again"))
                .on_hover_text("Re-runs identification for this package now")
                .clicked()
            {
                action = Some(Action::Retry(package.clone()));
            }
        });
    });
    action
}

/// Nothing to show, for one of two quite different reasons.
fn empty_state(
    ui: &mut egui::Ui,
    inbox: &crate::backend::inbox::InboxContents,
    filter: &retro_junk_backend::suggestions::SuggestionFilter,
) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        if inbox.pending_count() == 0 {
            ui.colored_label(crate::theme::STATUS_OK, "Nothing needs review.");
            ui.weak(
                "Confident, unambiguous work happens automatically; anything \
                 the tool should not decide for you shows up here.",
            );
        } else {
            ui.label(format!(
                "No reviews match {}.",
                if filter.pattern.is_empty() {
                    "that kind".to_owned()
                } else {
                    format!("'{}'", filter.pattern.source())
                }
            ));
            ui.weak(format!(
                "{} are waiting behind this filter.",
                inbox.pending_count()
            ));
        }
    });
}

/// Confirmations, the ignore-rule editor, and the candidate picker.
fn dialogs(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    confirm_dialog(ui, app, ctx);
    ignore_dialog(ui, app, ctx);
    choice_dialog(ui, app, ctx);
}

fn confirm_dialog(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    let Some(confirm) = &app.ui_state.inbox_ui.confirm else {
        return;
    };
    let count = confirm.ids.len();
    let description = confirm.description.clone();
    let ids = confirm.ids.clone();
    let dismissing = matches!(confirm.kind, InboxConfirmKind::Dismiss);
    let mut close = false;
    let mut go = false;
    egui::Window::new(if dismissing {
        "Dismiss these reviews?"
    } else {
        "Apply these suggestions?"
    })
    .collapsible(false)
    .resizable(false)
    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
    .show(ui.ctx(), |ui| {
        ui.label(format!("{count} review(s) — {description}"));
        ui.add_space(6.0);
        if dismissing {
            // The safety case for a button that closes hundreds of rows rests
            // on this being true, so it is stated rather than implied.
            ui.label("Dismissing closes review rows. It does not touch, move, or delete any file.");
            ui.label(
                "You can undo it immediately, and re-running adoption files anything still \
                 unaccounted for again — so this is not a permanent decision. To make it one, \
                 use \"Never ask again\" instead.",
            );
        } else {
            ui.label(
                "Each one is re-validated and then executed, one at a time. Anything that \
                 cannot be applied is reported and the rest still run.",
            );
        }
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .button(if dismissing {
                    format!("Dismiss {count}")
                } else {
                    format!("Apply {count}")
                })
                .clicked()
            {
                go = true;
            }
            if ui.button("Cancel").clicked() {
                close = true;
            }
        });
    });
    if go {
        if dismissing {
            crate::backend::inbox::dismiss(app, ids, ctx);
        } else {
            crate::backend::inbox::apply_many(app, ids, ctx);
        }
        app.ui_state.inbox_ui.confirm = None;
    } else if close {
        app.ui_state.inbox_ui.confirm = None;
    }
}

fn ignore_dialog(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    if app.ui_state.inbox_ui.ignore_draft.is_none() {
        return;
    }
    let mut close = false;
    let mut save = None;
    // Taken out so the dialog can hold a mutable borrow of the draft while the
    // rest of the app state stays reachable.
    let mut draft = app.ui_state.inbox_ui.ignore_draft.take().unwrap();
    // Recounted every frame against the pattern as it currently reads, so the
    // number in the dialog is always the number the button will act on.
    let covered = {
        let matcher = retro_junk_io::glob::Pattern::new(draft.pattern.trim());
        app.ui_state
            .inbox
            .items
            .iter()
            .filter(|item| {
                item.suggestion.kind == retro_junk_backend::ADOPT_SUGGESTION_KIND
                    && matcher.matches(&item.suggestion.target_id)
            })
            .count()
    };
    egui::Window::new("Never ask about these again")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label("Files matching this pattern are never filed for review again.");
            ui.horizontal(|ui| {
                ui.label("Pattern");
                ui.add(
                    egui::TextEdit::singleline(&mut draft.pattern)
                        .desired_width(280.0)
                        .hint_text("*.txt"),
                );
            });
            ui.horizontal(|ui| {
                ui.label("Note");
                ui.add(
                    egui::TextEdit::singleline(&mut draft.note)
                        .desired_width(280.0)
                        .hint_text("why, for later"),
                );
            });
            ui.add_space(6.0);
            ui.weak(format!(
                "Closes the {covered} review(s) it covers now, and stops the next sweep from \
                 hashing those files at all."
            ));
            ui.weak(
                "The rule is stored beside your collection, so it travels with it and \
                 survives the database being rebuilt. You can revoke it at any time.",
            );
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let valid = !draft.pattern.trim().is_empty();
                if ui
                    .add_enabled(valid, egui::Button::new("Ignore from now on"))
                    .on_disabled_hover_text("An empty pattern would cover the whole library")
                    .clicked()
                {
                    save = Some((
                        draft.pattern.trim().to_owned(),
                        draft.note.trim().to_owned(),
                    ));
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    let _ = ui;
    if let Some((pattern, note)) = save {
        crate::backend::inbox::ignore(app, pattern, note, ctx);
    } else if !close {
        app.ui_state.inbox_ui.ignore_draft = Some(draft);
    }
}

fn choice_dialog(ui: &mut egui::Ui, app: &mut RetroJunkApp, ctx: &egui::Context) {
    if app.ui_state.inbox_ui.choice.is_none() {
        return;
    }
    let mut choice = app.ui_state.inbox_ui.choice.take().unwrap();
    let mut close = false;
    let mut accept = None;
    egui::Window::new("Which one is it?")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
        .show(ctx, |ui| {
            ui.label(&choice.label);
            ui.weak("The tool found more than one answer and will not choose for you.");
            ui.add_space(6.0);
            for (index, candidate) in choice.candidates.iter().enumerate() {
                let selected = choice.selected == Some(index);
                if ui
                    .selectable_label(
                        selected,
                        format!("{}  ({})", candidate.label, candidate.kind.as_str()),
                    )
                    .clicked()
                {
                    choice.selected = Some(index);
                }
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let picked = choice
                    .selected
                    .and_then(|index| choice.candidates.get(index))
                    .map(|candidate| candidate.id.clone());
                if ui
                    .add_enabled(picked.is_some(), egui::Button::new("Accept"))
                    .clicked()
                    && let Some(id) = picked
                {
                    accept = Some(id);
                }
                if ui.button("Cancel").clicked() {
                    close = true;
                }
            });
        });
    let _ = ui;
    match accept {
        Some(candidate_id) => {
            let (id, label) = (choice.id, choice.label.clone());
            crate::backend::inbox::apply(app, id, Some(candidate_id), &label, ctx);
        }
        None if !close => app.ui_state.inbox_ui.choice = Some(choice),
        None => {}
    }
}

/// Arrow keys to move, Enter to apply, D to dismiss, E to expand.
///
/// Reviewing a queue is a rhythm; making every decision a mouse trip to a
/// different part of the row is what makes a backlog feel endless. Keys are
/// ignored while a text box has focus, or the filter box could not be typed
/// into.
fn keyboard(
    ui: &mut egui::Ui,
    app: &mut RetroJunkApp,
    filter: &retro_junk_backend::suggestions::SuggestionFilter,
    ctx: &egui::Context,
) {
    if ui.memory(egui::Memory::focused).is_some()
        || app.ui_state.inbox_ui.confirm.is_some()
        || app.ui_state.inbox_ui.ignore_draft.is_some()
        || app.ui_state.inbox_ui.choice.is_some()
    {
        return;
    }
    let order: Vec<i64> = app
        .ui_state
        .inbox
        .visible(filter, app.ui_state.inbox_ui.sort)
        .iter()
        .map(|item| item.suggestion.id)
        .collect();
    if order.is_empty() {
        return;
    }
    let position = app
        .ui_state
        .inbox_ui
        .cursor
        .and_then(|cursor| order.iter().position(|id| *id == cursor));
    let (down, up, expand, apply, dismiss) = ui.input(|input| {
        (
            input.key_pressed(egui::Key::ArrowDown) || input.key_pressed(egui::Key::J),
            input.key_pressed(egui::Key::ArrowUp) || input.key_pressed(egui::Key::K),
            input.key_pressed(egui::Key::E) || input.key_pressed(egui::Key::Space),
            input.key_pressed(egui::Key::Enter),
            input.key_pressed(egui::Key::D),
        )
    });
    if down {
        let next = position.map_or(0, |index| (index + 1).min(order.len() - 1));
        app.ui_state.inbox_ui.cursor = Some(order[next]);
        app.ui_state.inbox_ui.scroll_to_cursor = true;
        return;
    }
    if up {
        let next = position.map_or(0, |index| index.saturating_sub(1));
        app.ui_state.inbox_ui.cursor = Some(order[next]);
        app.ui_state.inbox_ui.scroll_to_cursor = true;
        return;
    }
    let Some(cursor) = app.ui_state.inbox_ui.cursor else {
        return;
    };
    if expand {
        if !app.ui_state.inbox_ui.expanded.remove(&cursor) {
            app.ui_state.inbox_ui.expanded.insert(cursor);
        }
        return;
    }
    if dismiss {
        crate::backend::inbox::dismiss(app, vec![cursor], ctx);
        return;
    }
    if apply {
        // Enter runs the obvious thing, and only the obvious thing: a review
        // with several answers has no obvious thing, so it opens the chooser
        // rather than picking one on the user's behalf.
        let Some(item) = app
            .ui_state
            .inbox
            .items
            .iter()
            .find(|item| item.suggestion.id == cursor)
        else {
            return;
        };
        if item.actions.choices.is_empty() {
            if item.actions.applicable {
                let label = item.headline.clone();
                crate::backend::inbox::apply(app, cursor, None, &label, ctx);
            }
        } else {
            app.ui_state.inbox_ui.choice = Some(InboxChoice {
                id: cursor,
                label: item.headline.clone(),
                candidates: item.actions.choices.clone(),
                selected: None,
            });
        }
    }
}

/// Carry out what the click asked for.
fn run(app: &mut RetroJunkApp, action: Action, ctx: &egui::Context) {
    match action {
        Action::Apply { id, label } => crate::backend::inbox::apply(app, id, None, &label, ctx),
        Action::Choose(choice) => app.ui_state.inbox_ui.choice = Some(choice),
        Action::Dismiss(ids) => crate::backend::inbox::dismiss(app, ids, ctx),
        Action::Reveal(path) => crate::util::reveal_in_file_manager(&path),
        Action::ToggleGroup(name) => {
            if !app.ui_state.inbox_ui.collapsed.remove(&name) {
                app.ui_state.inbox_ui.collapsed.insert(name);
            }
        }
        Action::ToggleRow(id) => {
            if !app.ui_state.inbox_ui.expanded.remove(&id) {
                app.ui_state.inbox_ui.expanded.insert(id);
            }
            app.ui_state.inbox_ui.cursor = Some(id);
        }
        Action::Focus(id) => {
            app.ui_state.inbox_ui.cursor = Some(id);
            if !app.ui_state.inbox_ui.expanded.remove(&id) {
                app.ui_state.inbox_ui.expanded.insert(id);
            }
        }
        Action::IgnoreOne(pattern) => {
            app.ui_state.inbox_ui.ignore_draft = Some(InboxIgnoreDraft {
                pattern,
                note: String::new(),
            });
        }
        Action::Retry(package) => crate::backend::inbox::retry_package(app, &package, ctx),
        Action::ForgetPackage(path) => crate::backend::inbox::forget_package(app, path, ctx),
    }
}

/// What the current filter is describing, in words, for a confirmation to
/// quote back.
fn describe_filter(app: &RetroJunkApp) -> String {
    let pattern = app.ui_state.inbox_ui.filter_text.trim();
    match (app.ui_state.inbox_ui.filter_kind.as_deref(), pattern) {
        (None, "") => "everything waiting".to_owned(),
        (None, pattern) => format!("matching '{pattern}'"),
        (Some(kind), "") => format!("every {}", kind_label(kind)),
        (Some(kind), pattern) => format!("{} matching '{pattern}'", kind_label(kind)),
    }
}

/// A suggestion kind as a person would say it.
fn kind_label(kind: &str) -> &str {
    match kind {
        "import" => "Imports",
        "scrape" => "Artwork",
        "adopt_playable" => "Unaccounted files",
        other => other,
    }
}

/// Confidence is stored as the weakest identification in the package; the
/// thresholds mirror `BindConfidence`'s ordering so the wording matches what
/// the automation policy would have done with it.
fn confidence_label(confidence: f64) -> &'static str {
    if confidence >= 1.0 {
        "exact hash"
    } else if confidence >= 0.7 {
        "header serial"
    } else if confidence >= 0.4 {
        "folder serial"
    } else {
        "unidentified"
    }
}

#[cfg(test)]
#[path = "inbox_tests.rs"]
mod tests;
