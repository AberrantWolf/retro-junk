//! Coalescer state machine: debounce collapse, modified-stickiness, rename
//! inference, and shutdown draining.

use std::path::PathBuf;
use std::sync::mpsc::channel;
use std::time::Duration;

use notify::EventKind;
use notify::event::{CreateKind, DataChange, ModifyKind, RemoveKind, RenameMode};

use super::coalesce::{Coalescer, DEBOUNCE};
use super::events::WatchEvent;

fn event(kind: EventKind, paths: &[&str]) -> notify::Event {
    let mut built = notify::Event::new(kind);
    for path in paths {
        built = built.add_path(PathBuf::from(path));
    }
    built
}

fn settle() {
    std::thread::sleep(DEBOUNCE + Duration::from_millis(50));
}

#[test]
fn rapid_writes_collapse_to_one_settled_event() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    for _ in 0..5 {
        coalescer.ingest(
            &event(
                EventKind::Modify(ModifyKind::Data(DataChange::Content)),
                &["/roms/a.nes"],
            ),
            &tx,
        );
    }
    assert!(coalescer.flush_expired(&tx));
    assert!(rx.try_recv().is_err(), "still inside the settle window");
    settle();
    assert!(coalescer.flush_expired(&tx));
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileModified(PathBuf::from("/roms/a.nes"))
    );
    assert!(rx.try_recv().is_err());
}

#[test]
fn modified_is_sticky_over_added() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    coalescer.ingest(
        &event(EventKind::Create(CreateKind::File), &["/in/a.bin"]),
        &tx,
    );
    coalescer.ingest(
        &event(
            EventKind::Modify(ModifyKind::Data(DataChange::Content)),
            &["/in/a.bin"],
        ),
        &tx,
    );
    // A late Create must not downgrade the pending Modified.
    coalescer.ingest(
        &event(EventKind::Create(CreateKind::File), &["/in/a.bin"]),
        &tx,
    );
    settle();
    assert!(coalescer.flush_expired(&tx));
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileModified(PathBuf::from("/in/a.bin"))
    );
}

#[test]
fn remove_cancels_pending_add_and_reports_removal() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    coalescer.ingest(
        &event(EventKind::Create(CreateKind::File), &["/in/tmp.bin"]),
        &tx,
    );
    coalescer.ingest(
        &event(EventKind::Remove(RemoveKind::File), &["/in/tmp.bin"]),
        &tx,
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileRemoved(PathBuf::from("/in/tmp.bin"))
    );
    settle();
    assert!(coalescer.flush_expired(&tx));
    assert!(rx.try_recv().is_err(), "the short-lived add never settles");
}

#[test]
fn paired_rename_emits_immediately() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    coalescer.ingest(
        &event(
            EventKind::Modify(ModifyKind::Name(RenameMode::Both)),
            &["/roms/old.nes", "/roms/new.nes"],
        ),
        &tx,
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileRenamed {
            from: PathBuf::from("/roms/old.nes"),
            to: PathBuf::from("/roms/new.nes"),
        }
    );
}

#[test]
fn remove_then_create_of_same_basename_is_a_rename() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    coalescer.ingest(
        &event(EventKind::Remove(RemoveKind::File), &["/a/game.nes"]),
        &tx,
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileRemoved(PathBuf::from("/a/game.nes"))
    );
    coalescer.ingest(
        &event(EventKind::Create(CreateKind::File), &["/b/game.nes"]),
        &tx,
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileRenamed {
            from: PathBuf::from("/a/game.nes"),
            to: PathBuf::from("/b/game.nes"),
        }
    );
}

#[test]
fn remove_then_create_of_different_basename_stays_separate() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    coalescer.ingest(
        &event(EventKind::Remove(RemoveKind::File), &["/a/one.nes"]),
        &tx,
    );
    coalescer.ingest(
        &event(EventKind::Create(CreateKind::File), &["/a/two.nes"]),
        &tx,
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileRemoved(PathBuf::from("/a/one.nes"))
    );
    settle();
    assert!(coalescer.flush_expired(&tx));
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileAdded(PathBuf::from("/a/two.nes"))
    );
}

#[test]
fn split_rename_pairs_by_tracker_id() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    let mut from = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::From)),
        &["/a/x.iso"],
    );
    from = from.set_tracker(7);
    coalescer.ingest(&from, &tx);
    let mut to = event(
        EventKind::Modify(ModifyKind::Name(RenameMode::To)),
        &["/b/y.iso"],
    );
    to = to.set_tracker(7);
    coalescer.ingest(&to, &tx);
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileRenamed {
            from: PathBuf::from("/a/x.iso"),
            to: PathBuf::from("/b/y.iso"),
        }
    );
}

#[test]
fn unpaired_rename_to_becomes_an_add() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    coalescer.ingest(
        &event(
            EventKind::Modify(ModifyKind::Name(RenameMode::To)),
            &["/a/appeared.nes"],
        ),
        &tx,
    );
    settle();
    assert!(coalescer.flush_expired(&tx));
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileAdded(PathBuf::from("/a/appeared.nes"))
    );
}

#[test]
fn directory_events_pass_straight_through() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    coalescer.ingest(
        &event(EventKind::Create(CreateKind::Folder), &["/in/dump-dir"]),
        &tx,
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::DirAdded(PathBuf::from("/in/dump-dir"))
    );
    coalescer.ingest(
        &event(EventKind::Remove(RemoveKind::Folder), &["/in/dump-dir"]),
        &tx,
    );
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::DirRemoved(PathBuf::from("/in/dump-dir"))
    );
}

#[test]
fn flush_all_drains_without_waiting() {
    let (tx, rx) = channel();
    let mut coalescer = Coalescer::new();
    coalescer.ingest(
        &event(EventKind::Create(CreateKind::File), &["/in/late.nes"]),
        &tx,
    );
    coalescer.flush_all(&tx);
    assert_eq!(
        rx.try_recv().unwrap(),
        WatchEvent::FileAdded(PathBuf::from("/in/late.nes"))
    );
}
