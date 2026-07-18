use super::*;

#[test]
fn log_entry_stores_fields() {
    let entry = LogEntry {
        timestamp: Local::now(),
        level: log::Level::Warn,
        target: "retro_junk_gui::scan".to_string(),
        message: "something happened".to_string(),
    };
    assert_eq!(entry.level, log::Level::Warn);
    assert_eq!(entry.target, "retro_junk_gui::scan");
    assert_eq!(entry.message, "something happened");
}

#[test]
fn ring_buffer_caps_at_max() {
    let mut buf: VecDeque<LogEntry> = VecDeque::with_capacity(MAX_ENTRIES);
    for i in 0..MAX_ENTRIES + 100 {
        if buf.len() >= MAX_ENTRIES {
            buf.pop_front();
        }
        buf.push_back(LogEntry {
            timestamp: Local::now(),
            level: log::Level::Info,
            target: "test".to_string(),
            message: format!("msg {i}"),
        });
    }
    assert_eq!(buf.len(), MAX_ENTRIES);
    // Oldest entry should be #100 (0-99 were evicted)
    assert_eq!(buf.front().unwrap().message, "msg 100");
}
