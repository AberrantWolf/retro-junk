use super::*;

#[test]
fn test_detect_z64() {
    assert_eq!(detect_n64_format(&MAGIC_Z64), Some(N64Format::Z64));
}

#[test]
fn test_detect_v64() {
    assert_eq!(detect_n64_format(&MAGIC_V64), Some(N64Format::V64));
}

#[test]
fn test_detect_n64() {
    assert_eq!(detect_n64_format(&MAGIC_N64), Some(N64Format::N64));
}

#[test]
fn test_detect_unknown() {
    assert_eq!(detect_n64_format(&[0xFF, 0xFF, 0xFF, 0xFF]), None);
}

#[test]
fn test_detect_too_short() {
    assert_eq!(detect_n64_format(&[0x80, 0x37]), None);
}

#[test]
fn test_normalize_z64_noop() {
    let original = vec![0x80, 0x37, 0x12, 0x40];
    let mut data = original.clone();
    normalize_to_big_endian(&mut data, N64Format::Z64);
    assert_eq!(data, original);
}

#[test]
fn test_normalize_v64() {
    let mut data = vec![0x37, 0x80, 0x40, 0x12];
    normalize_to_big_endian(&mut data, N64Format::V64);
    assert_eq!(data, vec![0x80, 0x37, 0x12, 0x40]);
}

#[test]
fn test_normalize_n64() {
    let mut data = vec![0x40, 0x12, 0x37, 0x80];
    normalize_to_big_endian(&mut data, N64Format::N64);
    assert_eq!(data, vec![0x80, 0x37, 0x12, 0x40]);
}
