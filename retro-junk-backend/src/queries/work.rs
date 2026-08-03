//! Questions about pending work: how much is waiting for the user, and
//! whether some other process has changed the database since we last looked.

use retro_junk_db::Connection;

/// Answers: "how many suggestions are still waiting for a decision?"
///
/// An unreadable work table answers zero: the badge is a hint, and a failure
/// to read it should never block the rest of the screen.
pub fn open_suggestion_count(conn: &Connection) -> u64 {
    retro_junk_db::work::list_open_suggestions(conn, None).map_or(0, |open| open.len() as u64)
}

/// Answers: "what is the database's current change counter?"
///
/// Every writer bumps this counter, so a frontend that polls it can notice
/// that another process (the daemon, or a second window) wrote something and
/// reload. `None` means the counter could not be read, which the caller should
/// treat as "no news" rather than as a change.
pub fn dirty_tick(conn: &Connection) -> Option<i64> {
    retro_junk_db::work::read_runtime_state(conn)
        .ok()
        .map(|runtime| runtime.dirty_tick)
}
