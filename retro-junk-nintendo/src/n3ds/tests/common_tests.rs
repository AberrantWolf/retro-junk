use super::*;
use std::io::Cursor;

#[test]
fn test_align64() {
    assert_eq!(align64(0), 0);
    assert_eq!(align64(1), 64);
    assert_eq!(align64(64), 64);
    assert_eq!(align64(65), 128);
    assert_eq!(align64(128), 128);
    assert_eq!(align64(0x2020), 0x2040);
}

#[test]
fn test_is_all_zeros() {
    assert!(is_all_zeros(&[0, 0, 0]));
    assert!(!is_all_zeros(&[0, 1, 0]));
    assert!(is_all_zeros(&[]));
}

#[test]
fn test_region_from_product_code() {
    assert_eq!(region_from_product_code("CTR-P-ABCE"), vec![Region::Usa]);
    assert_eq!(region_from_product_code("CTR-P-ABCJ"), vec![Region::Japan]);
    assert_eq!(region_from_product_code("CTR-P-ABCP"), vec![Region::Europe]);
    assert_eq!(region_from_product_code("CTR-P-ABCK"), vec![Region::Korea]);
    assert_eq!(region_from_product_code("CTR-P-ABCA"), vec![Region::World]);
}

#[test]
fn test_region_european_variants() {
    assert_eq!(region_from_product_code("CTR-P-ABCD"), vec![Region::Europe]);
    assert_eq!(region_from_product_code("CTR-P-ABCF"), vec![Region::Europe]);
    assert_eq!(region_from_product_code("CTR-P-ABCS"), vec![Region::Europe]);
    assert_eq!(region_from_product_code("CTR-P-ABCI"), vec![Region::Europe]);
    assert_eq!(region_from_product_code("CTR-P-ABCU"), vec![Region::Europe]);
}

#[test]
fn test_read_helpers_bounds_checking() {
    // Valid reads
    let buf = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
    assert_eq!(read_u16_le(&buf, 0), Some(0x0201));
    assert_eq!(read_u32_le(&buf, 0), Some(0x04030201));
    assert_eq!(read_u64_le(&buf, 0), Some(0x0807060504030201));
    assert_eq!(read_u16_be(&buf, 0), Some(0x0102));
    assert_eq!(read_u32_be(&buf, 0), Some(0x01020304));
    assert_eq!(read_u64_be(&buf, 0), Some(0x0102030405060708));

    // Out-of-bounds reads return None
    assert_eq!(read_u16_le(&buf, 7), None);
    assert_eq!(read_u32_le(&buf, 5), None);
    assert_eq!(read_u64_le(&buf, 1), None);
    assert_eq!(read_u16_be(&buf, 8), None);
    assert_eq!(read_u32_be(&buf, 6), None);
    assert_eq!(read_u64_be(&buf, 2), None);

    // Empty buffer
    assert_eq!(read_u16_le(&[], 0), None);
    assert_eq!(read_u32_le(&[0, 1], 0), None);
}

#[test]
fn test_sha256_verification() {
    use sha2::{Digest, Sha256};

    let data = vec![0x42u8; 256];
    let expected = {
        let mut hasher = Sha256::new();
        hasher.update(&data);
        let result = hasher.finalize();
        let mut arr = [0u8; 32];
        arr.copy_from_slice(&result);
        arr
    };

    // Valid hash
    let mut cursor = Cursor::new(data.clone());
    match verify_sha256(&mut cursor, 0, 256, &expected).unwrap() {
        HashResult::Ok => {}
        other => panic!("Expected Ok, got {:?}", matches!(other, HashResult::Ok)),
    }

    // Invalid hash (corrupt data)
    let mut bad_data = data;
    bad_data[0] = 0x00;
    let mut cursor = Cursor::new(bad_data);
    match verify_sha256(&mut cursor, 0, 256, &expected).unwrap() {
        HashResult::Mismatch { .. } => {}
        other => panic!(
            "Expected Mismatch, got {:?}",
            matches!(other, HashResult::Ok)
        ),
    }
}
