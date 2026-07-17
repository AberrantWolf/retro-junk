use super::*;

#[test]
fn test_serial_attempts_both_different() {
    let attempts = serial_attempts("NUS-NSME-USA", "NSME");
    assert_eq!(attempts, vec!["NSME", "NUS-NSME-USA"]);
}

#[test]
fn test_serial_attempts_same_value() {
    let attempts = serial_attempts("SLUS-01234", "SLUS-01234");
    assert_eq!(attempts, vec!["SLUS-01234"]);
}

#[test]
fn test_serial_attempts_no_scraper_serial() {
    let attempts = serial_attempts("NUS-NSME-USA", "");
    assert_eq!(attempts, vec!["NUS-NSME-USA"]);
}

#[test]
fn test_serial_attempts_no_serial_at_all() {
    let attempts = serial_attempts("", "");
    assert!(attempts.is_empty());
}
