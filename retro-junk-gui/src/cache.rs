// The row ↔ entry conversions live in the backend's `library` module beside
// the entry model itself; re-exported so existing `crate::cache::` callers
// keep working. Nothing else remains: this file used to hold the one-time
// migration from the pre-SQLite JSON cache, which described state that is
// rebuilt from disk by any scan.
pub(crate) use retro_junk_backend::library::{
    detail_to_entry, entry_analysis_update, entry_hash_update,
};
#[cfg(test)]
pub(crate) use retro_junk_backend::library::{entry_to_row, row_to_entry};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::DiscVerification;

    #[test]
    fn standalone_disc_integrity_and_warnings_survive_row_round_trip() {
        let mut entry = crate::test_support::test_entry(
            retro_junk_lib::scanner::GameEntry::SingleFile("game.cue".into()),
        );
        entry.hashes = Some(retro_junk_dat::FileHashes {
            crc32: "12345678".into(),
            sha1: Some("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into()),
            md5: None,
            data_size: 2352,
            warnings: vec!["Incomplete disc: DAT Track 2 is missing".into()],
        });
        entry.disc_verification = DiscVerification::Incomplete;

        let restored = row_to_entry(entry_to_row(&entry).unwrap()).unwrap();

        assert_eq!(restored.disc_verification, DiscVerification::Incomplete);
        assert_eq!(
            restored.hashes.unwrap().warnings,
            vec!["Incomplete disc: DAT Track 2 is missing"]
        );
    }
}
