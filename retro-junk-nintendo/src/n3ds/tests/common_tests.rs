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
fn test_media_type_name() {
    assert_eq!(media_type_name(0), "Inner Device");
    assert_eq!(media_type_name(1), "Card1");
    assert_eq!(media_type_name(2), "Card2");
}

#[test]
fn test_maker_codes() {
    assert_eq!(maker_code_name("31"), Some("Nintendo"));
    assert_eq!(maker_code_name("SQ"), Some("Square Enix"));
    assert_eq!(maker_code_name("NB"), Some("Bandai Namco"));
    assert_eq!(maker_code_name("ZZ"), None);
}

#[test]
fn test_title_id_format() {
    assert_eq!(format_title_id(0x0004000000ABCDEF), "0004000000ABCDEF");
}

#[test]
fn test_title_type() {
    assert_eq!(title_type_from_id(0x0004000000000000), "Application");
    assert_eq!(title_type_from_id(0x0004000100000000), "System Application");
    assert_eq!(title_type_from_id(0x0004008C00000000), "DLC");
    assert_eq!(title_type_from_id(0x0004000E00000000), "Patch/Update");
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
