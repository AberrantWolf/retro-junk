/// Result of processing keyboard navigation input.
pub struct NavAction {
    /// The new index to navigate to.
    pub new_index: usize,
    /// Whether Shift was held (for extending selection).
    pub extend_selection: bool,
}

/// Process arrow/home/end/page keyboard input for list navigation.
///
/// Consumes matching key events from `ui` and returns a `NavAction` if a
/// navigation key was pressed. Returns `None` if count is 0 or no nav key
/// was pressed.
pub fn process_list_nav(
    ui: &mut egui::Ui,
    current: Option<usize>,
    count: usize,
    page_size: usize,
) -> Option<NavAction> {
    if count == 0 {
        return None;
    }

    let (key, shift) = try_consume_nav_key(ui)?;
    let idx = current.unwrap_or(0);
    let new_index = compute_new_index(key, idx, count, page_size);

    Some(NavAction {
        new_index,
        extend_selection: shift,
    })
}

/// Navigation keys we handle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NavKey {
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
}

/// Try to consume a navigation key from the UI input. Returns the key and
/// whether Shift was held.
fn try_consume_nav_key(ui: &mut egui::Ui) -> Option<(NavKey, bool)> {
    // Check with and without shift for each key
    let keys = [
        (egui::Key::ArrowUp, NavKey::Up),
        (egui::Key::ArrowDown, NavKey::Down),
        (egui::Key::Home, NavKey::Home),
        (egui::Key::End, NavKey::End),
        (egui::Key::PageUp, NavKey::PageUp),
        (egui::Key::PageDown, NavKey::PageDown),
    ];

    for (egui_key, nav_key) in keys {
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::SHIFT, egui_key)) {
            return Some((nav_key, true));
        }
        if ui.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui_key)) {
            return Some((nav_key, false));
        }
    }

    None
}

/// Pure index computation — testable without egui context.
pub(crate) fn compute_new_index(
    key: NavKey,
    current: usize,
    count: usize,
    page_size: usize,
) -> usize {
    match key {
        NavKey::Up => current.saturating_sub(1),
        NavKey::Down => (current + 1).min(count - 1),
        NavKey::Home => 0,
        NavKey::End => count - 1,
        NavKey::PageUp => current.saturating_sub(page_size),
        NavKey::PageDown => (current + page_size).min(count - 1),
    }
}

#[cfg(test)]
#[path = "keyboard_nav_tests.rs"]
mod tests;
