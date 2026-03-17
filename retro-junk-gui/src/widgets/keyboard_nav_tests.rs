use super::{NavKey, compute_new_index};

#[test]
fn arrow_down_from_zero() {
    assert_eq!(compute_new_index(NavKey::Down, 0, 10, 5), 1);
}

#[test]
fn arrow_up_from_zero_stays() {
    assert_eq!(compute_new_index(NavKey::Up, 0, 10, 5), 0);
}

#[test]
fn arrow_down_at_end_clamps() {
    assert_eq!(compute_new_index(NavKey::Down, 9, 10, 5), 9);
}

#[test]
fn arrow_up_from_middle() {
    assert_eq!(compute_new_index(NavKey::Up, 5, 10, 5), 4);
}

#[test]
fn home_jumps_to_start() {
    assert_eq!(compute_new_index(NavKey::Home, 7, 10, 5), 0);
}

#[test]
fn end_jumps_to_last() {
    assert_eq!(compute_new_index(NavKey::End, 3, 10, 5), 9);
}

#[test]
fn page_down_normal() {
    assert_eq!(compute_new_index(NavKey::PageDown, 2, 20, 5), 7);
}

#[test]
fn page_down_near_end_clamps() {
    assert_eq!(compute_new_index(NavKey::PageDown, 8, 10, 5), 9);
}

#[test]
fn page_up_normal() {
    assert_eq!(compute_new_index(NavKey::PageUp, 7, 20, 5), 2);
}

#[test]
fn page_up_near_start_clamps() {
    assert_eq!(compute_new_index(NavKey::PageUp, 2, 10, 5), 0);
}

#[test]
fn single_item_list() {
    assert_eq!(compute_new_index(NavKey::Down, 0, 1, 5), 0);
    assert_eq!(compute_new_index(NavKey::Up, 0, 1, 5), 0);
    assert_eq!(compute_new_index(NavKey::Home, 0, 1, 5), 0);
    assert_eq!(compute_new_index(NavKey::End, 0, 1, 5), 0);
}

#[test]
fn page_size_larger_than_list() {
    assert_eq!(compute_new_index(NavKey::PageDown, 0, 3, 10), 2);
    assert_eq!(compute_new_index(NavKey::PageUp, 2, 3, 10), 0);
}
