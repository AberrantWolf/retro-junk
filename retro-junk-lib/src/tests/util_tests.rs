use super::*;

#[test]
fn test_format_bytes() {
    assert_eq!(format_bytes(0), "0 bytes");
    assert_eq!(format_bytes(512), "512 bytes");
    assert_eq!(format_bytes(1024), "1 KB");
    assert_eq!(format_bytes(4096), "4 KB");
    assert_eq!(format_bytes(1048576), "1 MB");
    assert_eq!(format_bytes(4194304), "4 MB");
    assert_eq!(format_bytes(1025), "1025 bytes");
}
