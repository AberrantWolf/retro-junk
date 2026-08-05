//! Pick which catalog entry an ambiguous file is.
//!
//! The list is the entry's own candidates and nothing else. Offering a free
//! search here would let a person assert an identity the evidence rules out,
//! and every downstream decision — the canonical name, the artwork queried
//! for, the disc set expected — would then be built on that assertion.
//!
//! The choice is written as a content-keyed mark beside the collection, so it
//! survives a rename and a rebuilt database. It never makes the entry read as
//! verified: it reads as asserted, and stays re-selectable.

use crate::app::RetroJunkApp;

/// What the dialog is deciding about.
#[derive(Debug, Clone)]
pub struct DisambiguatePrompt {
    pub label: String,
    pub platform_id: String,
    pub content: retro_junk_archive::MarkedContent,
    pub candidates: Vec<retro_junk_db::identify::Candidate>,
    /// The choice already recorded, if this is a re-selection.
    pub chosen: Option<String>,
    /// Which row is highlighted, as an index into `candidates`.
    pub selected: usize,
}

pub fn show(ctx: &egui::Context, app: &mut RetroJunkApp) {
    if app.ui_state.disambiguate_prompt.is_none() {
        return;
    }
    let mut apply = false;
    let mut clear = false;
    let mut close = false;
    let mut selected = app
        .ui_state
        .disambiguate_prompt
        .as_ref()
        .map_or(0, |prompt| prompt.selected);

    let dismissed = {
        let prompt = app.ui_state.disambiguate_prompt.as_ref().unwrap().clone();
        crate::widgets::modal::show(
            ctx,
            "disambiguate_dialog",
            "Which game is this?",
            560.0,
            |ui| {
                ui.label(format!(
                    "The hashes for {} match more than one catalog entry, so the tool will not \
                     guess. These are the entries it cannot rule out:",
                    prompt.label
                ));
                ui.add_space(6.0);
                for (index, candidate) in prompt.candidates.iter().enumerate() {
                    // Region, revision and variant are exactly what separates
                    // candidates that share a name, so the row has to show
                    // them or the choice is between identical-looking lines.
                    let line = [
                        candidate.game.as_str(),
                        candidate.region.as_str(),
                        candidate.revision.as_str(),
                        candidate.variant.as_str(),
                    ]
                    .iter()
                    .filter(|part| !part.is_empty())
                    .copied()
                    .collect::<Vec<_>>()
                    .join("  ·  ");
                    ui.radio_value(&mut selected, index, line);
                }
                ui.add_space(6.0);
                ui.weak(
                    "A hand-picked entry is never reported as verified — it stays marked as \
                     chosen by you, and you can change it later. Hashing the remaining tracks \
                     would settle it without a choice.",
                );
                ui.add_space(6.0);
                crate::widgets::modal::footer(ui, |ui| {
                    if ui.button("Use this entry").clicked() {
                        apply = true;
                    }
                    if prompt.chosen.is_some() && ui.button("Forget my choice").clicked() {
                        clear = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            },
        )
        .dismissed
    };

    if let Some(prompt) = app.ui_state.disambiguate_prompt.as_mut() {
        prompt.selected = selected;
    }

    if apply {
        let prompt = app.ui_state.disambiguate_prompt.clone().unwrap();
        if let Some(candidate) = prompt.candidates.get(prompt.selected) {
            record_choice(app, &prompt, &candidate.media_id, &candidate.game);
        }
        app.ui_state.disambiguate_prompt = None;
    } else if clear {
        let prompt = app.ui_state.disambiguate_prompt.clone().unwrap();
        forget_choice(app, &prompt);
        app.ui_state.disambiguate_prompt = None;
    } else if close || dismissed {
        app.ui_state.disambiguate_prompt = None;
    }
}

/// Write the choice through the one store, and say so.
fn record_choice(
    app: &mut RetroJunkApp,
    prompt: &DisambiguatePrompt,
    media_id: &str,
    dat_name: &str,
) {
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        app.push_error("Choose entry", "No active collection profile");
        return;
    };
    match retro_junk_backend::disambiguation::choose(
        &profile.collection_root(),
        &prompt.platform_id,
        &prompt.content,
        media_id,
        dat_name,
    ) {
        Ok(()) => app.notify(format!("{} is now {dat_name}", prompt.label)),
        Err(error) => app.push_error("Choose entry", &error),
    }
}

fn forget_choice(app: &mut RetroJunkApp, prompt: &DisambiguatePrompt) {
    let Some(profile) = app.settings.library.active_profile().cloned() else {
        app.push_error("Choose entry", "No active collection profile");
        return;
    };
    match retro_junk_backend::disambiguation::clear(
        &profile.collection_root(),
        &prompt.platform_id,
        &prompt.content,
    ) {
        Ok(_) => app.notify(format!("{} is ambiguous again", prompt.label)),
        Err(error) => app.push_error("Choose entry", &error),
    }
}
