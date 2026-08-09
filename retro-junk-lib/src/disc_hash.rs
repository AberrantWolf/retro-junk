//! Logical per-track hashing for CUE/BIN disc sets.
//!
//! Redump fingerprints optical discs per track.  Hashing only the first data
//! track is useful for identification, but it is not sufficient to verify a
//! complete disc.  This module is the normal-library counterpart to the
//! maintenance verifier: it maps both split-BIN and combined-BIN CUE layouts
//! into logical track spans and hashes every span exactly once.

use std::collections::HashSet;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use retro_junk_core::{AnalysisError, FileHashes, HashAlgorithms, MultiHasher};

#[derive(Debug, Clone)]
pub struct DiscTrackHashes {
    pub track_number: u8,
    pub is_data: bool,
    pub hashes: FileHashes,
}

#[derive(Debug, Clone)]
pub struct CueDiscHashes {
    pub primary: FileHashes,
    pub tracks: Vec<DiscTrackHashes>,
}

/// Hash every logical track described by a standard CUE sheet.
///
/// Progress is aggregate logical bytes across the complete disc, regardless
/// of whether the tracks live in one combined BIN or separate files.
pub fn hash_cue_disc(
    cue_path: &Path,
    progress: &dyn Fn(u64, u64),
) -> Result<CueDiscHashes, AnalysisError> {
    let cue_text = std::fs::read_to_string(cue_path)?;
    let sheet = retro_junk_disc::cue::parse_cue(&cue_text)?;
    let cue_dir = cue_path.parent().unwrap_or(Path::new("."));
    let spans = retro_junk_disc::track_layout::cue_track_spans(&sheet, cue_dir)?;

    if spans.is_empty() {
        return Err(AnalysisError::invalid_format("CUE describes no tracks"));
    }

    let mut track_modes = std::collections::HashMap::new();
    let mut seen_numbers = HashSet::new();
    let mut previous_number = None;
    for file in &sheet.files {
        for track in &file.tracks {
            if !seen_numbers.insert(track.number) {
                return Err(AnalysisError::invalid_format(format!(
                    "CUE declares TRACK {:02} more than once",
                    track.number
                )));
            }
            if previous_number.is_some_and(|previous| track.number <= previous) {
                return Err(AnalysisError::invalid_format(
                    "CUE track numbers are not strictly increasing",
                ));
            }
            previous_number = Some(track.number);
            track_modes.insert(track.number, track.mode.clone());
        }
    }

    if spans.iter().any(|span| span.byte_len == 0) {
        return Err(AnalysisError::invalid_format(
            "CUE describes an empty logical track",
        ));
    }

    let total = spans.iter().map(|span| span.byte_len).sum();
    let mut completed = 0_u64;
    let mut tracks = Vec::with_capacity(spans.len());

    for span in spans {
        let mut file = File::open(&span.file)?;
        file.seek(SeekFrom::Start(span.byte_offset))?;
        let base = completed;
        let per_track = |done: u64, _track_total: u64| progress(base.saturating_add(done), total);
        let mut hasher = MultiHasher::new(HashAlgorithms::All, span.byte_len, Some(&per_track));
        let mut remaining = span.byte_len;
        let mut buffer = vec![0_u8; 64 * 1024];
        while remaining > 0 {
            let wanted = remaining.min(buffer.len() as u64) as usize;
            let read = file.read(&mut buffer[..wanted])?;
            if read == 0 {
                return Err(AnalysisError::other(format!(
                    "Unexpected end of {} while hashing TRACK {:02}",
                    span.file.display(),
                    span.track_number
                )));
            }
            hasher.update_with_progress(&buffer[..read]);
            remaining -= read as u64;
        }
        completed = completed.saturating_add(span.byte_len);
        let mode = track_modes
            .get(&span.track_number)
            .map_or("", String::as_str);
        tracks.push(DiscTrackHashes {
            track_number: span.track_number,
            is_data: !mode.eq_ignore_ascii_case("AUDIO"),
            hashes: hasher.finalize(),
        });
    }

    let primary = tracks
        .iter()
        .find(|track| track.is_data)
        .or_else(|| tracks.first())
        .ok_or_else(|| AnalysisError::invalid_format("CUE describes no hashable tracks"))?
        .hashes
        .clone();

    Ok(CueDiscHashes { primary, tracks })
}

/// Hash every logical track stored in a CD CHD.
pub fn hash_chd_disc(
    chd_path: &Path,
    progress: &dyn Fn(u64, u64),
) -> Result<CueDiscHashes, AnalysisError> {
    let mut file = std::io::BufReader::with_capacity(8 * 1024 * 1024, File::open(chd_path)?);
    let tracks = retro_junk_disc::hash_chd_tracks(&mut file, HashAlgorithms::All, Some(progress))?
        .into_iter()
        .map(|track| DiscTrackHashes {
            track_number: u8::try_from(track.track_number).unwrap_or(u8::MAX),
            is_data: track.is_data,
            hashes: track.hashes,
        })
        .collect::<Vec<_>>();
    let primary = tracks
        .iter()
        .filter(|track| track.is_data)
        .max_by_key(|track| track.hashes.data_size)
        .or_else(|| tracks.first())
        .ok_or_else(|| AnalysisError::invalid_format("CHD describes no hashable tracks"))?
        .hashes
        .clone();
    Ok(CueDiscHashes { primary, tracks })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combined_bin_is_hashed_as_distinct_logical_tracks() {
        let dir = tempfile::tempdir().unwrap();
        let cue = dir.path().join("game.cue");
        let bin = dir.path().join("game.bin");
        let track_1 = vec![0x11_u8; 2 * 2352];
        let track_2 = vec![0x22_u8; 3 * 2352];
        std::fs::write(&bin, [track_1.as_slice(), track_2.as_slice()].concat()).unwrap();
        std::fs::write(
            &cue,
            "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\n  TRACK 02 AUDIO\n    INDEX 01 00:00:02\n",
        )
        .unwrap();

        let progress = std::cell::Cell::new((0, 0));
        let result = hash_cue_disc(&cue, &|done, total| progress.set((done, total))).unwrap();

        assert_eq!(result.tracks.len(), 2);
        assert_eq!(result.tracks[0].hashes.data_size, track_1.len() as u64);
        assert_eq!(result.tracks[1].hashes.data_size, track_2.len() as u64);
        assert_eq!(result.primary.crc32, result.tracks[0].hashes.crc32);
        assert_eq!(
            progress.get(),
            (bin.metadata().unwrap().len(), bin.metadata().unwrap().len())
        );
    }

    #[test]
    fn empty_orphan_file_directive_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let cue = dir.path().join("game.cue");
        std::fs::write(dir.path().join("game.bin"), vec![0_u8; 2352]).unwrap();
        std::fs::write(
            &cue,
            "FILE \"game.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"game.bin\" BINARY\n",
        )
        .unwrap();

        assert!(hash_cue_disc(&cue, &|_, _| {}).is_err());
    }

    #[test]
    fn split_bin_tracks_are_each_hashed_in_full() {
        let dir = tempfile::tempdir().unwrap();
        let cue = dir.path().join("game.cue");
        std::fs::write(dir.path().join("track1.bin"), vec![1_u8; 2352]).unwrap();
        std::fs::write(dir.path().join("track2.bin"), vec![2_u8; 2 * 2352]).unwrap();
        std::fs::write(
            &cue,
            "FILE \"track1.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"track2.bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\n",
        )
        .unwrap();

        let result = hash_cue_disc(&cue, &|_, _| {}).unwrap();
        assert_eq!(result.tracks[0].hashes.data_size, 2352);
        assert_eq!(result.tracks[1].hashes.data_size, 2 * 2352);
    }

    #[test]
    fn missing_referenced_track_is_rejected_before_hashing() {
        let dir = tempfile::tempdir().unwrap();
        let cue = dir.path().join("game.cue");
        std::fs::write(dir.path().join("track1.bin"), vec![1_u8; 2352]).unwrap();
        std::fs::write(
            &cue,
            "FILE \"track1.bin\" BINARY\n  TRACK 01 MODE2/2352\n    INDEX 01 00:00:00\nFILE \"missing.bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\n",
        )
        .unwrap();

        assert!(hash_cue_disc(&cue, &|_, _| {}).is_err());
    }

    #[test]
    fn chd_hashes_the_same_complete_track_set_as_its_source_cue() {
        if std::process::Command::new("chdman")
            .arg("--help")
            .output()
            .is_err()
        {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let cue = dir.path().join("game.cue");
        let chd = dir.path().join("game.chd");
        std::fs::write(dir.path().join("track1.bin"), vec![0x11_u8; 3 * 2352]).unwrap();
        std::fs::write(dir.path().join("track2.bin"), vec![0x22_u8; 5 * 2352]).unwrap();
        std::fs::write(
            &cue,
            "FILE \"track1.bin\" BINARY\n  TRACK 01 MODE1/2352\n    INDEX 01 00:00:00\nFILE \"track2.bin\" BINARY\n  TRACK 02 AUDIO\n    INDEX 01 00:00:00\n",
        )
        .unwrap();
        let status = std::process::Command::new("chdman")
            .args(["createcd", "-i"])
            .arg(&cue)
            .arg("-o")
            .arg(&chd)
            .status()
            .unwrap();
        assert!(status.success());

        let source = hash_cue_disc(&cue, &|_, _| {}).unwrap();
        let compressed = hash_chd_disc(&chd, &|_, _| {}).unwrap();
        assert_eq!(compressed.tracks.len(), source.tracks.len());
        for (actual, expected) in compressed.tracks.iter().zip(&source.tracks) {
            assert_eq!(actual.track_number, expected.track_number);
            assert_eq!(actual.hashes.data_size, expected.hashes.data_size);
            assert_eq!(actual.hashes.sha1, expected.hashes.sha1);
        }
    }
}
