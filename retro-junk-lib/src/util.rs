/// Format a byte size as a human-readable string.
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 && bytes.is_multiple_of(1024 * 1024) {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} bytes", bytes)
    }
}

#[cfg(test)]
mod tests {
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
}
