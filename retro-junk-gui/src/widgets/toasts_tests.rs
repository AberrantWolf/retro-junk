use std::time::{Duration, Instant};

use super::{FADE, HOLD, Toasts};

#[test]
fn a_repeated_message_refreshes_instead_of_stacking() {
    let mut toasts = Toasts::default();
    toasts.success("Built Game.chd");
    toasts.success("Built Game.chd");
    assert_eq!(
        toasts.items.len(),
        1,
        "batch operations reporting the same completion must not bury the screen"
    );
}

#[test]
fn toasts_expire_after_the_hold_and_fade() {
    let mut toasts = Toasts::default();
    toasts.success("Organized psx");
    let raised = toasts.items[0].raised;

    assert!(toasts.retain_live(raised + HOLD));
    assert!(toasts.retain_live(raised + HOLD + FADE / 2));
    assert!(!toasts.retain_live(raised + HOLD + FADE + Duration::from_millis(1)));
}

#[test]
fn opacity_fades_from_one_to_zero_across_the_fade_window() {
    let raised = Instant::now();
    let mut toasts = Toasts::default();
    toasts.success("done");
    toasts.items[0].raised = raised;

    let item = &toasts.items[0];
    assert_eq!(item.opacity(raised), Some(1.0));
    let half = item.opacity(raised + HOLD + FADE / 2).unwrap();
    assert!(
        (0.4..=0.6).contains(&half),
        "expected roughly half opacity mid-fade, got {half}"
    );
    assert_eq!(item.opacity(raised + HOLD + FADE), None);
}
