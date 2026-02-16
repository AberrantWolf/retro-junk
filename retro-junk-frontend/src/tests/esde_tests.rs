use super::*;

#[test]
fn test_format_esde_date() {
    assert_eq!(format_esde_date("1996-06-23"), "19960623T000000");
    assert_eq!(format_esde_date("19960623"), "19960623T000000");
}

#[test]
fn test_escape_xml() {
    assert_eq!(escape_xml("Tom & Jerry"), "Tom &amp; Jerry");
    assert_eq!(escape_xml("a < b"), "a &lt; b");
}
