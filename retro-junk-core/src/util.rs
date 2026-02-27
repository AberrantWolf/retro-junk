/// Format a byte count as a human-readable size string (e.g., "4 KB", "2 MB").
///
/// Uses exact integer division — values that aren't clean multiples of KB/MB
/// are shown in bytes. For approximate/fractional display, see [`format_bytes_approx`].
pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 && bytes.is_multiple_of(1024 * 1024) {
        format!("{} MB", bytes / (1024 * 1024))
    } else if bytes >= 1024 && bytes.is_multiple_of(1024) {
        format!("{} KB", bytes / 1024)
    } else {
        format!("{} bytes", bytes)
    }
}

/// Format a byte count with fractional KB/MB (e.g., "1.5 KB", "2.3 MB").
///
/// Better for cache/file sizes where exact binary alignment isn't guaranteed.
pub fn format_bytes_approx(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{} B", bytes)
    }
}

/// Read a null-terminated ASCII string from a byte slice.
///
/// Stops at the first null byte, filters out non-printable characters,
/// and returns the result. No trimming is performed.
pub fn read_ascii(buf: &[u8]) -> String {
    buf.iter()
        .take_while(|&&b| b != 0)
        .filter(|&&b| (0x20..0x7F).contains(&b))
        .map(|&b| b as char)
        .collect()
}

/// Read a fixed-length ASCII string from a byte slice.
///
/// Non-printable bytes are replaced with spaces, then the result is trimmed.
/// Unlike [`read_ascii`], this does NOT stop at null bytes — it processes
/// the entire buffer. Useful for ROM headers where fields are padded
/// with 0x00 or 0xFF rather than null-terminated.
pub fn read_ascii_fixed(buf: &[u8]) -> String {
    let s: String = buf
        .iter()
        .map(|&b| {
            if (0x20..0x7F).contains(&b) {
                b as char
            } else {
                ' '
            }
        })
        .collect();
    s.trim().to_string()
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
        assert_eq!(format_bytes(16384), "16 KB");
        assert_eq!(format_bytes(262144), "256 KB");
        assert_eq!(format_bytes(1048576), "1 MB");
        assert_eq!(format_bytes(4194304), "4 MB");
        assert_eq!(format_bytes(1025), "1025 bytes");
    }

    #[test]
    fn test_read_ascii() {
        assert_eq!(read_ascii(b"HELLO\0WORLD"), "HELLO");
        assert_eq!(read_ascii(b"\x01\x02ABC"), "ABC");
        assert_eq!(read_ascii(b""), "");
        assert_eq!(read_ascii(b"\0"), "");
    }

    #[test]
    fn test_read_ascii_fixed() {
        assert_eq!(read_ascii_fixed(b"HELLO\0\0\0"), "HELLO");
        assert_eq!(read_ascii_fixed(b"\xFF\xFFABC\xFF\xFF"), "ABC");
        assert_eq!(read_ascii_fixed(b"  PADDED  "), "PADDED");
    }

    #[test]
    fn test_format_bytes_approx() {
        assert_eq!(format_bytes_approx(0), "0 B");
        assert_eq!(format_bytes_approx(512), "512 B");
        assert_eq!(format_bytes_approx(1024), "1.0 KB");
        assert_eq!(format_bytes_approx(1536), "1.5 KB");
        assert_eq!(format_bytes_approx(1048576), "1.0 MB");
    }
}
