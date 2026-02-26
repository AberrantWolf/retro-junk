use super::*;

#[test]
fn test_serial_attempts_both_different() {
    let serial = Some("NUS-NSME-USA".to_string());
    let scraper = Some("NSME".to_string());
    let attempts = serial_attempts(&serial, &scraper);
    assert_eq!(attempts, vec!["NSME", "NUS-NSME-USA"]);
}

#[test]
fn test_serial_attempts_same_value() {
    let serial = Some("SLUS-01234".to_string());
    let scraper = Some("SLUS-01234".to_string());
    let attempts = serial_attempts(&serial, &scraper);
    assert_eq!(attempts, vec!["SLUS-01234"]);
}

#[test]
fn test_serial_attempts_no_scraper_serial() {
    let serial = Some("NUS-NSME-USA".to_string());
    let scraper = None;
    let attempts = serial_attempts(&serial, &scraper);
    assert_eq!(attempts, vec!["NUS-NSME-USA"]);
}

#[test]
fn test_serial_attempts_no_serial_at_all() {
    let serial: Option<String> = None;
    let scraper: Option<String> = None;
    let attempts = serial_attempts(&serial, &scraper);
    assert!(attempts.is_empty());
}
