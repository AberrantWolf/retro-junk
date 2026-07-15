# CHD Compression & Disc-Pipeline Remediation — Implementation Spec

Status: **approved for implementation** (derived from the 2026-07-14 multi-agent code review of the uncommitted CHD-compression change set).

This spec is self-contained: every work item states the current behavior with exact
file/line anchors, the required behavior, concrete implementation steps (signatures
included), and tests. Line numbers refer to the working tree as of the review; re-locate
by the quoted identifiers if lines have drifted.

**Severity legend:** `P0` = data loss / silent corruption, `P1` = wrong results or
unusable feature, `P2` = robustness / UX, `P3` = maintainability / DRY.

---

## 0. Implementation order and dependency graph

Implement in phase order. Within a phase, items are independent unless noted.

```
Phase A  retro-junk-disc foundations (parsers, spans)
   A1 sector-size unification ──┐
   A2 MSF overflow              │
   A3 parser strictness         ├─→ Phase B depends on A1, A4, A5
   A4 PREGAP representation     │
   A5 span coverage             │
   A6 GDI format documentation ─┘
Phase B  retro-junk-lib::chd_convert core safety
   B1 output ownership (temp-file protocol)   ← the P0 data-loss fix
   B2 responsive cancellation
   B3 byte-compare verification (replaces hash_span)
   B4 track-number pairing
   B5 m3u rewrite via rename.rs
   B6 typed skip classes + plan_batch + finalize_verified
   B7 Chdman::detect_from_setting helper
Phase C  retro-junk-core trait + analyzers
   C1 declarative chd_extensions table (replaces 6 overrides)  ← B6 consumes this
   C2 container-hash gaps (Sega CD, Dreamcast, PSP) + invariant test
   C3 ChdMedia documentation cleanup
Phase D  retro-junk-gui
   D1 async planning & chdman probe        ← uses B6, B7
   D2 worker shutdown (cancel + join)      ← uses B2
   D3 overlapping-operation guard
   D4 completion-handler rework            ← uses B6 result shape
   D5 refresh_multidisc_files via scanner
   D6 generic results dialog + status colors
   D7 ProgressDisplay enum
   D8 confirmation-dialog rendering perf
   D9 settings save-on-focus-loss
Phase E  retro-junk-cli
   E1 skip reporting via plan_batch        ← uses B6
   E2 CliError::ExternalTool variant
   E3 progress-message throttling; error-tail cleanup
Phase F  deferred / structural (document in TODO.md, do not implement now)
```

Recommended commit granularity: one commit per work item (A1..E3), phases in order,
`cargo test` green after every commit.

---

## Phase A — retro-junk-disc foundations

### A1. Unify mode→sector-size logic (P1, DRY) — `cue.rs`, `track_layout.rs`, `hash.rs`

**Current behavior.** Three divergent implementations:

1. `retro-junk-disc/src/cue.rs:424` — private `fn sector_size_for_mode(mode: &str) -> u16`,
   the canonical table (`MODE2_FORM1`→2048, `MODE2`/`MODE2_FORM_MIX`→2336,
   `MODE2_FORM2`→2324, `AUDIO`/`*_RAW`→2352, default 2352). Does **not** parse
   `MODE1/2352`-style slash suffixes generically (only listed literals).
2. `retro-junk-disc/src/track_layout.rs:34` — `fn cue_sector_size(mode: &str) -> u64`:
   parses `X/NNNN` suffix, else returns 2352. **Disagrees** with (1) for every
   slashless CDRWin mode (`MODE2_FORM1` → 2352 instead of 2048, etc.).
   `parse_cue` (cue.rs:56) deliberately passes CDRWin modes through verbatim, and the
   compression path (`chd_convert::layout_spans` → `parse_cue` → `cue_track_spans`)
   never runs the cue-fix conversion, so CDRWin cues get wrong byte spans → false
   round-trip mismatches (good CHD deleted) or misaligned hashing.
3. `retro-junk-disc/src/hash.rs:415` — `compute_track1_size_from_cue` hardcodes
   `RAW_SECTOR_SIZE` (2352), ignoring the declared mode; wrong track-1 boundary for
   `MODE2/2336` / `MODE1/2048` single-bin rips.

**Required behavior.** Exactly one implementation, mode-aware everywhere.

**Implementation.**

1. In `cue.rs`, replace the private function with:

   ```rust
   /// Sector size in bytes for a cue TRACK mode string (standard or CDRWin).
   ///
   /// Standard modes carry the size after the slash (`MODE1/2352`, `MODE2/2336`);
   /// CDRWin modes are looked up by name. Unknown modes default to raw (2352).
   pub fn sector_size_for_mode(mode: &str) -> u64 {
       if let Some((_, size)) = mode.rsplit_once('/')
           && let Ok(n) = size.trim().parse::<u64>()
       {
           return n;
       }
       match mode.to_uppercase().as_str() {
           "MODE1" | "MODE2_FORM1" => 2048,          // CDRWin cooked
           "MODE2_FORM2" => 2324,
           "MODE2" | "MODE2_FORM_MIX" => 2336,
           "MODE1_RAW" | "MODE2_RAW" | "AUDIO" => 2352,
           _ => 2352,
       }
   }
   ```

   Note the return type changes `u16 → u64`; update the existing in-crate callers
   (grep `sector_size_for_mode` in `cue.rs` — used by the cue-compat/fix code) with
   plain casts where a `u16` was consumed. The added bare-`"MODE1"` arm is CDRWin
   knowledge; per CLAUDE.md, record the source (CDRWin/cdrdao cue-sheet documentation)
   in `.claude/skills/retro-archive/formats/` alongside the existing CUE notes when
   making this change.

2. In `track_layout.rs`, delete `cue_sector_size` (lines 30–38) and import
   `crate::cue::sector_size_for_mode`; call sites at lines 82 and 86.

3. In `hash.rs::compute_track1_size_from_cue` (line 390–429): derive the size from the
   file's tracks instead of `RAW_SECTOR_SIZE`:

   ```rust
   let sector_size = crate::cue::sector_size_for_mode(&file.tracks[0].mode);
   if file.tracks.iter().any(|t| crate::cue::sector_size_for_mode(&t.mode) != sector_size) {
       warnings.push("CUE mixes sector sizes within one file; cannot derive Track 1 boundary".into());
       return (None, warnings);
   }
   let track1_size = idx01.to_sector_offset() * sector_size;
   ```

**Tests** (`retro-junk-disc/src/tests/track_layout_tests.rs` + cue tests):
- `sector_size_for_mode`: `"MODE1/2352"`→2352, `"MODE2/2048"`→2048, `"MODE2_FORM1"`→2048,
  `"MODE2"`→2336, `"MODE2_FORM2"`→2324, `"AUDIO"`→2352, `"MODE1"`→2048, garbage→2352,
  `"MODE2/abc"`→2336 (falls through to name lookup — verify chosen semantics and pin them).
- `cue_track_spans` over a CDRWin-mode cue (`TRACK MODE2_FORM1`) computes 2048-byte spans.
- `compute_track1_size_from_cue` with a `MODE2/2336` cue returns `sector*2336`.

### A2. MSF arithmetic overflow (P2) — `cue.rs:44`

**Current.** `((self.minutes * 60 + self.seconds) as u64) * 75 + ...` multiplies in
`u32`; `minutes ≥ 71_582_789` (parseable from a malformed cue) panics in debug, wraps
in release → garbage span.

**Fix.**

```rust
pub fn to_sector_offset(&self) -> u64 {
    (self.minutes as u64 * 60 + self.seconds as u64) * 75 + self.frames as u64
}
```

**Test:** `CueIndex { number: 1, minutes: u32::MAX, seconds: 59, frames: 74 }`
returns the mathematically correct u64 without panicking.

### A3. Parser strictness: tabs and malformed INDEX lines (P1) — `cue.rs`

**Current.**
- Directive detection (`cue.rs:71-73`, `:103`, `:117`) requires a literal ASCII space
  (`upper.starts_with("FILE ")` …), so tab-separated cue lines are silently skipped:
  a single-FILE tab cue errors "contains no FILE entries"; in multi-FILE cues, tracks
  attach to the wrong `CueFile` → wrong spans.
- `cue.rs:119`: `if let Ok(index) = parse_cue_index_line(line)` swallows the parse
  error, so e.g. `INDEX 01 54:04.52` (period — real exporter quirk) is dropped,
  surfacing later as a misleading "TRACK 02 has no INDEX lines" or a wrong span.

**Fix.**

1. Detect directives by first whitespace-delimited token instead of prefix+space.
   At the top of the per-line handling, compute once:

   ```rust
   let keyword = line.split_whitespace().next().unwrap_or("").to_uppercase();
   let rest = line[keyword.len()..].trim_start();   // keyword is at start post-trim
   ```

   Then branch on `keyword.as_str()`: `"FILE" | "DATAFILE" | "AUDIOFILE"`, `"TRACK"`,
   `"INDEX"`, `"PREGAP"`, `"POSTGAP"` (see A4). Rework `parse_cue_file_line_at` to
   take `rest` directly instead of a `skip_len` byte offset (delete the skip_len
   computation at cue.rs:80-88). `parse_cue_track_line` / `parse_cue_index_line`
   already tokenize with `split_whitespace` and keep working on the full `line`.

2. Propagate INDEX parse errors: replace the `if let Ok(...)` at cue.rs:119 with `?`,
   and enrich `parse_cue_index_line`'s error messages to include the offending line
   text (`format!("Invalid MSF timestamp in CUE INDEX: {line}")`). A cue that lies
   about its indexes must fail loudly — a silent drop now feeds destructive
   verify-then-delete logic.

**Tests:**
- Tab-separated cue (`"FILE\t\"a.bin\"\tBINARY\nTRACK\t01\tMODE1/2352\nINDEX\t01\t00:00:00"`)
  parses identically to the space-separated version.
- `INDEX 01 54:04.52` → `Err` naming the line.
- Existing cue tests still pass (no behavior change for well-formed sheets).

### A4. Represent PREGAP/POSTGAP; reject them at compression planning (P1) — `cue.rs`, `chd_convert.rs`

**Current.** `cue.rs:129` ignores `PREGAP`/`POSTGAP`; `CueTrack` (cue.rs:25) cannot
represent them; a repo-wide grep confirms no compensation anywhere. For
directive-pregap rips (TOSEC/CDRWin style — gap **not** stored in the bin), chdman
synthesizes the gap into the CHD and materializes it on extraction, so extracted track
lengths exceed source spans → `verify_round_trip` reports Mismatch → the (perfectly
good) CHD is deleted and the disc is permanently uncompressible, after wasting a full
compress+extract cycle each attempt.

**Required.** Parse and represent the directives; **fail at plan time** with an
accurate reason instead of failing verification after minutes of work. (Gap
compensation during verification is a possible future enhancement — record it in
TODO.md, do not build it now.)

**Implementation.**

1. `cue.rs`: add to `CueTrack`:

   ```rust
   /// Frames of pregap declared via a PREGAP directive (gap data NOT stored
   /// in the file). 0 when absent. In-file pregaps use INDEX 00 instead.
   pub pregap_frames: u64,
   /// Frames declared via POSTGAP (not stored in the file). 0 when absent.
   pub postgap_frames: u64,
   ```

   Initialize to 0 at both construction sites (cue.rs:106 and the CueTrack in
   pending_tracks). Parse in the A3 keyword match: `PREGAP`/`POSTGAP` take an
   `MM:SS:FF` argument; factor the MSF-string parsing out of `parse_cue_index_line`
   into `fn parse_msf(s: &str) -> Result<(u32, u32, u32), AnalysisError>` and reuse it
   (also usable by `msf_to_sectors`, cue.rs:439 — fold that in too). Attach to the
   current last track exactly like INDEX attachment; a PREGAP with no current track is
   an `invalid_format` error.

2. `chd_convert::plan_compression` `"cue"` branch (chd_convert.rs:193): after
   `expand_disc_set`, additionally parse the cue
   (`retro_junk_disc::cue::parse_cue(&fs::read_to_string(input)?)?`) and reject:

   ```rust
   if sheet.files.iter().flat_map(|f| &f.tracks)
        .any(|t| t.pregap_frames > 0 || t.postgap_frames > 0)
   {
       return Err(ChdConvertError::UnsupportedLayout {
           detail: "CUE declares PREGAP/POSTGAP gaps that are not stored in the \
                    track files; chdman synthesizes them into the CHD, so a \
                    byte-exact round-trip comparison is impossible".to_string(),
       });
   }
   ```

   New error variant:

   ```rust
   #[error("cannot verify layout: {detail}")]
   UnsupportedLayout { detail: String },
   ```

   The parsed sheet is also what B1/B4 verification re-parses — do not cache it across
   the chdman run (the extracted side has its own cue), but do pass it to
   `cue_track_spans` here if convenient to avoid a second parse within planning.

**Tests:** cue with `PREGAP 00:02:00` on track 2 → `plan_compression` returns
`UnsupportedLayout` (build a minimal analyzer stub mapping `"cue"`→`Cd`, as the
existing `chd_convert_tests.rs` does); `parse_cue` populates `pregap_frames = 150`.

### A5. Track spans must tile the whole file (P0) — `track_layout.rs:94-115`

**Current.** Each track's span starts at its **first INDEX** (`starts[0]` for the
first track), so bytes before track 1's first index (e.g. `TRACK 01` declaring only
`INDEX 01 00:02:00` — 150 frames of in-file data, no `INDEX 00`) are covered by **no
span**. Verification passes without ever reading them; with delete-sources on, that
data is silently lost. The only bounds check is `start > end || end > file_size`
(line 102) — there is no coverage check.

**Required.** Within each `CueFile`, spans must exactly tile `0..file_size`.

**Implementation.** In `cue_track_spans`:

1. Force the first track's span of each file to start at byte 0:

   ```rust
   let mut starts = Vec::with_capacity(file.tracks.len());
   for track in &file.tracks {
       starts.push(first_index_frame(track)? * sector_size);
   }
   // INDEX offsets are within-file; anything before track 1's first index is
   // track 1's implicit pregap region and must be owned by its span.
   starts[0] = 0;
   ```

2. This makes coverage total by construction (`starts[0] == 0`, last span ends at
   `file_size`, spans are contiguous). Keep the existing `start > end` check — it now
   also rejects out-of-order INDEX times.

3. Update the module doc (lines 9–12): the "matches `chdman extractcd --splitbin`"
   claim is inaccurate (verification runs plain `extractcd` and re-parses the
   extracted cue, so only *symmetry* between the two sides matters, not chdman's
   internal split rule). Rewrite to:

   > Track boundaries: within each FILE, track 1 owns everything from byte 0; each
   > subsequent track starts at its first INDEX (00 when present, else 01). Spans
   > always tile the entire file, so round-trip verification covers every byte.
   > The same rule applies to the source cue and to the cue chdman writes on
   > extraction, which is what makes the comparison sound.

**Tests:**
- Cue with `TRACK 01 / INDEX 01 00:02:00` in a bin sized 10 sectors: track 1's span is
  `0 .. file_size_of_track1_region` (starts at 0, not 150 frames in).
- Sum of `byte_len` over each file's spans equals the file size (add this as a generic
  assertion to every existing span test).

### A6. Document the GDI format knowledge (P3, CLAUDE.md compliance)

**Current.** `parse_gdi` (track_layout.rs:137) encodes the GDI descriptor grammar
(track-count first line; `number lba type sector_size filename offset` track lines;
double-quoted filenames; high-density area at LBA 45000) but nothing exists in
`.claude/skills/retro-archive/formats/`, violating the repo rule that file-format
knowledge must be documented there with credited sources.

**Fix.** Create `.claude/skills/retro-archive/formats/GDI.md` covering: file purpose
(GD-ROM track list for Dreamcast rips), the line grammar exactly as implemented,
field meanings (`type` 4 = data / 0 = audio; `sector_size` 2048 or 2352; `offset`
usually 0), the LBA 45000 high-density convention, quoting rules, and the
relationship to CHD (`chdman createcd` accepts `.gdi` input). Credit sources: the
chdman source (`src/tools/chdman.cpp` in the MAME repository) and the existing
implementation-verification notes in `formats/CHD.md`; mark any statement not yet
verified against an authoritative source as such.

---

## Phase B — chd_convert core safety

### B1. Output ownership: never delete a CHD this job did not create (P0 — the data-loss fix) — `chd_convert.rs:291-341`

**Current.** `plan_compression` checks `output.exists()` only at plan time
(line 184–186); `compress_to_chd` passes no overwrite flag and re-checks nothing; on
**any** chdman failure it runs `remove_file_best_effort(&job.output)` (line 315).
Consequence: same-stem `Game.cue` + `Game.iso` (PS2 maps both extensions) both plan
`Game.chd`; job 1 succeeds (and with delete-sources removes cue+bin); job 2's chdman
refuses to overwrite and exits non-zero; the error path **deletes job 1's verified
CHD**. Originals and CHD both gone. Also reachable via any `.chd` appearing between
plan and execution.

**Required.** The failure path may only ever delete files this job itself created.

**Implementation — temp-output publish protocol.**

1. In `compress_to_chd`, compute a temp output in the same directory (same
   filesystem, so the final rename is atomic):

   ```rust
   let stem = job.output.file_stem().and_then(|s| s.to_str()).unwrap_or("chd");
   let temp_output = job.output.with_file_name(format!(".{stem}.chd.tmp"));
   if temp_output.exists() {
       fs::remove_file(&temp_output)?;   // stale leftover from a crashed run
   }
   ```

2. Run `createcd`/`createdvd` with `-o temp_output`. Every failure path removes
   `temp_output` (never `job.output`).

3. `verify_round_trip` gains a `chd_path: &Path` parameter and is called with
   `&temp_output` (replace the two uses of `job.output` inside it: the extract `-i`
   argument at line 461, and the temp-dir naming at 433–442 — the temp-dir stem can
   keep using `job.output`'s stem).

4. On `Verified`: publish —

   ```rust
   if job.output.exists() {
       // Appeared since planning (another tool, another run). Do not clobber.
       fs::remove_file(&temp_output)?;
       return Err(ChdConvertError::OutputExists(job.output.clone()));
   }
   fs::rename(&temp_output, &job.output)?;
   ```

   Read `output_bytes` from `job.output` after the rename. On `Mismatch`: remove
   `temp_output` (as today's semantics, but on the temp file).

5. The residual check-then-rename race is closed by B6 (batch-level duplicate-output
   rejection) and D3 (no overlapping runs); note this in a comment at the recheck.

**Tests** (`chd_convert_tests.rs`; use the existing fake-chdman/script-based test
infrastructure if present, else unit-test the path arithmetic and the
`OutputExists`-on-publish branch by pre-creating `job.output` between plan and a
mocked verify):
- Failure after a pre-existing `Game.chd` never removes it.
- Successful pipeline leaves no `.{stem}.chd.tmp` behind.
- Publish with `job.output` pre-created → `Err(OutputExists)`, temp removed,
  pre-existing file untouched (byte-identical).

### B2. Responsive cancellation in `run_chdman` (P2) — `chd_convert.rs:600-661`

**Current.** The loop checks `cancel` only at the top of each iteration, then blocks
in `stderr.read()` (line 627) with no timeout. If chdman stalls without emitting
stderr (hung network mount, spun-down disk), Cancel never reaches `child.kill()`;
the worker thread hangs forever and the GUI operation can never be cancelled.

**Implementation.** Move the blocking read to a dedicated reader thread; poll on the
main side:

```rust
let (chunk_tx, chunk_rx) = std::sync::mpsc::channel::<Vec<u8>>();
let mut stderr = child.stderr.take().expect("stderr piped");
let reader = std::thread::spawn(move || {
    let mut buf = [0u8; 4096];
    while let Ok(n) = stderr.read(&mut buf) {
        if n == 0 { break; }
        if chunk_tx.send(buf[..n].to_vec()).is_err() { break; }
    }
});

let mut cancelled = false;
loop {
    if cancel.load(Ordering::Relaxed) && !cancelled {
        cancelled = true;
        let _ = child.kill();          // reader unblocks via EOF after kill
    }
    match chunk_rx.recv_timeout(std::time::Duration::from_millis(100)) {
        Ok(chunk) => { /* existing byte-splitting into handle_chdman_line */ }
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
    }
}
handle_chdman_line(&pending, on_percent, &mut info_lines);
let _ = reader.join();
let status = child.wait()?;           // child killed or exited: returns promptly
```

Keep the existing `cancelled → Err(Cancelled)`, non-zero-status, and info-line-tail
logic. While here, apply E3's tail simplification (see E3) since the code is under
edit.

**Tests:** with a fake "chdman" script that ignores SIGTERM-free sleep (e.g.
`sleep 30` emitting nothing), setting `cancel` returns `Err(Cancelled)` within ~1s
(guard with a generous test timeout; mark `#[ignore]` if the suite must stay
hermetic on Windows).

### B3. Verification by byte comparison, not double hashing (P2 perf; kills the `expect`) — `chd_convert.rs:499-582`

**Current.** `hash_span` instantiates `MultiHasher::new(HashAlgorithms::Crc32Sha1, ..)`
but consumes only `.sha1.expect("Crc32Sha1 always produces sha1")` — an unused CRC32
is computed over source **and** extracted data (2× total disc size), and the
`expect` encodes a cross-module invariant. Hashing both sides at all is unnecessary
for an equality check: lengths are already fast-checked (line 488), and a streaming
compare can early-exit on the first differing chunk.

**Implementation.** Replace `hash_span` with:

```rust
/// Stream-compare two equal-length spans. Returns Ok(true) when every byte
/// matches; early-exits on the first differing chunk.
fn spans_equal(
    a: &TrackSpan,
    b: &TrackSpan,
    compared_bytes: &mut u64,
    total_bytes: u64,
    on_progress: ChdProgressFn<'_>,
    cancel: &AtomicBool,
) -> Result<bool, ChdConvertError>
```

Open both files, seek to the respective offsets, loop with two 1 MiB buffers:
read-exact `want = remaining.min(1 MiB)` from each (loop on short reads exactly as
the current EOF handling does, erroring `UnexpectedEof` naming the file), `memcmp`
(`buf_a[..want] != buf_b[..want]` → `Ok(false)`), update `compared_bytes` by `want`
(count each span-pair's bytes **once**: set `total_bytes = sum(byte_len)`, not
`* 2`, and adjust the caller at line 499), report `ChdPhase::Verifying` progress,
check `cancel` per chunk. In `verify_round_trip`, replace the two `hash_span` calls
(lines 502-509) with one `spans_equal` call per pair. Remove the now-unused
`MultiHasher`/`HashAlgorithms` imports (line 24).

**Tests:** equal spans → true; single-byte difference in the middle of a >1 MiB span
→ false (and, via a counting progress callback, confirm early exit — bytes compared
< total); differing lengths never reach `spans_equal` (guarded by the caller).

### B4. Pair verification spans by track number (P1) — `chd_convert.rs:488-510`

**Current.** Source and extracted spans are `zip`ped by position; `track_number` is
used only in error text. `cue_track_spans` emits spans in cue FILE/TRACK listing
order, so a cue listing tracks out of ascending order is compared
track-against-wrong-track → false mismatch → good CHD deleted (and the disc can
never compress).

**Fix.** In `verify_round_trip`, after computing both span lists:

```rust
let mut source_spans = source_spans;
let mut extracted_spans = extracted_spans;
source_spans.sort_by_key(|s| s.track_number);
extracted_spans.sort_by_key(|s| s.track_number);
if source_spans.iter().map(|s| s.track_number).ne(extracted_spans.iter().map(|e| e.track_number)) {
    return Ok(VerificationOutcome::Mismatch {
        detail: "track numbering differs between source and extracted CHD".to_string(),
    });
}
```

(Keep the existing count and per-track length checks; they now compare like tracks.)

**Test:** two span lists with tracks `[2, 1]` vs `[1, 2]` and per-track distinct
content verify correctly after sorting (unit-test the sort+pair logic; a full
out-of-order cue fixture is optional).

### B5. Rewrite `.m3u` playlists via the existing rename.rs machinery (P1, DRY) — `chd_convert.rs:360-402`, `rename.rs`

**Current.** `update_m3u_references` matches only `line.trim() == old_name` —
case-sensitive, no `./` or subdirectory prefixes — so prefixed/case-differing
playlist entries survive untouched; after `delete_job_sources` removes the cue the
playlist dangles and the multi-disc game no longer loads. Meanwhile
`rename.rs:2221 fix_m3u_references_in_dir` → `fix_references_in_dir(&M3uFormat, ..)`
already implements playlist rewriting with materially better matching
(`find_correct_m3u_entry`, rename.rs:2328: rename-map lookup, then same-stem with
original-extension-first fallback over `M3U_ENTRY_POINT_EXTENSIONS`).

**Implementation.**

1. In `rename.rs`, widen visibility: `pub(crate) fn fix_m3u_references_in_dir(...)`.
2. Verify `M3U_ENTRY_POINT_EXTENSIONS` contains `"chd"`; add it if missing (this is
   what lets the stem fallback resolve `Game (Disc 1).cue` → `Game (Disc 1).chd`
   after the cue is deleted, including for `./`-prefixed entries whose exact name
   misses the rename map).
3. Replace the body of `update_m3u_references`:

   ```rust
   /// Rewrite sibling `.m3u` playlists that referenced the job's input file to
   /// point at the new `.chd`. Call after [`delete_job_sources`].
   pub fn update_m3u_references(job: &CompressionJob) -> (usize, Vec<String>) {
       let dir = job.input.parent().unwrap_or(Path::new("."));
       let (Some(old_name), Some(new_name)) = (
           job.input.file_name().and_then(|n| n.to_str()),
           job.output.file_name().and_then(|n| n.to_str()),
       ) else { return (0, Vec::new()); };
       let rename_map = std::collections::HashMap::from([(old_name.to_string(), new_name.to_string())]);
       let mut errors = Vec::new();
       let updated = crate::rename::fix_m3u_references_in_dir(dir, &rename_map, &mut errors);
       (updated, errors)
   }
   ```

   Update the two callers (GUI `backend/chd_compress.rs:165`, CLI
   `commands/compress.rs:213`) for the new return type: log `errors` as warnings.
4. Known residual gap (document in a comment, do not fix here): on case-**sensitive**
   filesystems a playlist entry with different case than the actual file still
   misses; extending `find_correct_m3u_entry` with a case-insensitive directory probe
   is a TODO.md item (Phase F list).

**Tests** (in `chd_convert_tests.rs` or rename tests): playlist containing
`./Game (Disc 1).cue` and a directory holding `Game (Disc 1).chd` (cue deleted) →
line rewritten to `Game (Disc 1).chd`; plain-name entries still rewritten; comment
lines (`#…`) untouched.

### B6. Typed skip classes, batch planner, and shared finalize (P1/P3) — `chd_convert.rs`, both frontends

**Current.**
- `plan_compression`'s hint (chd_convert.rs:169-176) hardcodes console-specific
  extensions (`"cso" | "dax" | "cdi" | "gcz" | "rvz"`) in retro-junk-lib, violating
  "no console-specific code in lib".
- Both frontends re-classify `UnsupportedSource` by string-matching extensions with
  **divergent** policies: the CLI (compress.rs:89-96) counts `extension == "chd"` and
  silently drops everything else — an all-`.cso` PSP folder ends with just
  "Nothing compressed."; the GUI (backend/chd_compress.rs:71-72) suppresses only
  `"bin" | "img"` inside multi-disc entries.
- The verified-success sequence (delete sources → update m3u → aggregate failures) is
  duplicated in both frontends (compress.rs:211-221, chd_compress.rs:163-182).

**Implementation.**

1. Restructure the error variant (replaces the free-text `hint`):

   ```rust
   /// Why a file cannot be compressed to CHD. Display text derives from this;
   /// frontends branch on it instead of string-matching extensions.
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum SourceSkipClass {
       /// Already a .chd.
       AlreadyChd,
       /// Track/data file referenced by a sheet; compress the sheet instead.
       CompanionData,
       /// A container format chdman cannot read (declared by the analyzer —
       /// e.g. CSO/DAX, CDI). See RomAnalyzer::chd_extensions (C1).
       UnreadableContainer,
       /// No conversion is defined for this platform+extension.
       NoConversion,
   }

   #[error("{platform} cannot compress .{extension} files to CHD{}", class.hint())]
   UnsupportedSource { platform: String, extension: String, class: SourceSkipClass },
   ```

   with `impl SourceSkipClass { fn hint(&self) -> &'static str }` producing the
   current messages generically (`" (already a CHD)"`,
   `" (compress the .cue/.gdi that references it instead)"`,
   `" (a container format chdman cannot read)"`, `""`). Classification inside
   `plan_compression`: `"chd"` → `AlreadyChd` (format-generic); analyzer table role
   `Unconvertible` (C1) → `UnreadableContainer`; `"bin" | "img"` → `CompanionData`
   (optical-disc-generic); else `NoConversion`. The console-specific extension list
   in lib is thereby deleted; `gcz`/`rvz` disappear entirely (no analyzer declares
   them — GameCube/Wii do not use CHD).

2. Batch planning, one implementation for both frontends:

   ```rust
   pub struct PlanSkip {
       pub input: PathBuf,
       pub error: ChdConvertError,     // carries SourceSkipClass when applicable
   }
   pub struct PlannedBatch {
       pub jobs: Vec<CompressionJob>,
       /// Skips worth telling the user about (everything except CompanionData).
       pub skips: Vec<PlanSkip>,
       pub already_chd: usize,
   }

   /// Plan every input, classify skips, and reject duplicate outputs: if two
   /// inputs (e.g. same-stem cue+iso) map to one .chd, the first wins and the
   /// second becomes a skip (guards the B1 protocol at plan time).
   pub fn plan_batch(inputs: &[PathBuf], analyzer: &dyn RomAnalyzer) -> PlannedBatch
   ```

   Duplicate-output detection: `HashSet<PathBuf>` of planned outputs; a colliding job
   becomes a skip with a new variant
   `#[error("output {} already planned for another input in this batch", .0.display())] DuplicateOutput(PathBuf)`.
   `AlreadyChd` increments the counter instead of joining `skips`;
   `CompanionData` is dropped silently (both frontends agreed it is noise —
   this is now the *single* place that policy lives).

3. Shared post-verify step:

   ```rust
   pub struct FinalizeReport {
       pub sources_deleted: bool,
       pub delete_failures: Vec<(PathBuf, String)>,
       pub m3u_lines_updated: usize,
       pub m3u_errors: Vec<String>,
   }
   /// After a verified compression: optionally delete sources, then repoint
   /// sibling .m3u playlists at the new .chd.
   pub fn finalize_verified(job: &CompressionJob, delete_sources: bool) -> FinalizeReport
   ```

   Both frontends call this instead of hand-rolling delete+m3u (GUI
   chd_compress.rs:163-182; CLI compress.rs:211-221).

**Tests:** `plan_batch` over `[Game.cue, Game.iso]` with a PS2-style stub analyzer
yields 1 job + 1 `DuplicateOutput` skip; `.cso` input with a table declaring
`("cso", Unconvertible)` classifies `UnreadableContainer`; loose `Game (Track 2).bin`
→ silent (not in `skips`); `.chd` → counted.

### B7. One chdman-detection entry point for settings strings (P3) — `chd_convert.rs`, GUI

**Current.** The trim/empty-check/`PathBuf::from` dance around `Chdman::detect` is
duplicated at backend/chd_compress.rs:81-83 and views/settings.rs:176-184.

**Fix.** Add to `impl Chdman`:

```rust
/// Detect chdman from a settings string: empty/whitespace means "use PATH".
pub fn detect_from_setting(setting: &str) -> Result<Chdman, ChdmanUnavailable> {
    let trimmed = setting.trim();
    let override_path = (!trimmed.is_empty()).then(|| PathBuf::from(trimmed));
    Self::detect(override_path.as_deref())
}
```

Use it at both GUI sites (which D1 moves onto worker threads). The CLI keeps
`detect(chdman_override.as_deref())` — its input is already `Option<PathBuf>` from
clap.

---

## Phase C — retro-junk-core trait + analyzers

### C1. Declarative CHD extension table (P3 → prevents future P1s) — `retro-junk-core/src/lib.rs:376`, six analyzers, GUI

**Current.** Six structurally identical `chd_media_for_extension` match blocks
(dreamcast.rs:47, saturn.rs:353, sega_cd.rs:47, ps1.rs:304, ps2.rs:289, psp.rs:48);
the GUI additionally hardcodes a probe list `["cue", "gdi", "iso"]`
(backend/chd_compress.rs:27) because the query-only method cannot enumerate — a
seventh platform or a new extension requires touching N call sites, silently.

**Implementation.**

1. In core, next to `ChdMedia`:

   ```rust
   /// Role of a file extension in CHD conversion, declared per analyzer.
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum ChdExtensionRole {
       /// chdman can convert this source losslessly to CHD.
       Source(ChdMedia),
       /// A container/image format chdman cannot read (CSO, DAX, CDI, ...);
       /// declared so frontends can explain *why* instead of staying silent.
       Unconvertible,
   }
   ```

2. On `RomAnalyzer`, replace the required-override pattern with data + a provided
   lookup:

   ```rust
   /// Extensions relevant to CHD conversion for this platform. Lowercase,
   /// no dot. Default: none (cartridge platforms).
   fn chd_extensions(&self) -> &'static [(&'static str, ChdExtensionRole)] { &[] }

   /// Provided: the chdman media type for a convertible extension.
   fn chd_media_for_extension(&self, extension: &str) -> Option<ChdMedia> {
       self.chd_extensions().iter().find_map(|(e, role)| match role {
           ChdExtensionRole::Source(m) if *e == extension => Some(*m),
           _ => None,
       })
   }
   ```

   (Keep `chd_media_for_extension` callable everywhere it is today; it is now
   provided, not overridden.)

3. Replace the six overrides with tables (delete the match blocks):

   | analyzer | `chd_extensions()` |
   |---|---|
   | dreamcast.rs | `&[("gdi", Source(Cd)), ("cdi", Unconvertible)]` |
   | saturn.rs | `&[("cue", Source(Cd))]` |
   | sega_cd.rs | `&[("cue", Source(Cd))]` |
   | ps1.rs | `&[("cue", Source(Cd))]` |
   | ps2.rs | `&[("cue", Source(Cd)), ("iso", Source(Dvd))]` |
   | psp.rs | `&[("iso", Source(Dvd)), ("cso", Unconvertible), ("dax", Unconvertible)]` |

   Preserve the existing per-platform doc comments (e.g. psp.rs:49-50) on the table.

4. GUI `console_supports_chd` (backend/chd_compress.rs:22-31): delete the probe list;
   gate on

   ```rust
   rc.analyzer.chd_extensions().iter()
       .any(|(_, role)| matches!(role, ChdExtensionRole::Source(_)))
   ```

5. `plan_compression` consults the table for `Unconvertible` classification (B6).
   Note the `_ => {}` fallthrough at chd_convert.rs:215 and the non-cue/gdi arm of
   `layout_spans` (chd_convert.rs:530): with the table it is now *provable* which
   extensions reach them; add a debug assertion or comment that any `Source(Cd)`
   extension other than `cue`/`gdi` and any `Source(Dvd)` extension other than `iso`
   requires extending `plan_compression`/`layout_spans` — and make
   `plan_compression` return `UnsupportedLayout` (A4's variant) instead of silently
   treating an unknown sheet-like source as a single file:

   ```rust
   ("cue" | "gdi" | "iso") => { /* existing handling */ }
   other => return Err(ChdConvertError::UnsupportedLayout {
       detail: format!(".{other} is declared CHD-convertible but no track-layout \
                        handling exists for it"),
   }),
   ```

**Tests:** table lookup (`chd_media_for_extension("cue")` per analyzer);
`console_supports_chd`-equivalent logic true for the six disc analyzers, false for a
cartridge analyzer; planning a hypothetical `Source`-declared unknown extension →
`UnsupportedLayout`, not a bogus single-file job.

### C2. Close the container-hashing gaps; enforce the invariant (P0) — `sega_cd.rs`, `dreamcast.rs`, `psp.rs`, new test

**Current.** `sega_cd.rs:32` and `dreamcast.rs:32` list `"chd"` in
`file_extensions()` but do not override `compute_container_hashes`; the core default
returns `Ok(None)` (core lib.rs:364) and `hasher.rs:71-84` falls through to streaming
the raw container bytes — so a CHD produced by this very feature hashes to garbage
and can never match Redump (violates CLAUDE.md: "Never hash compressed container
bytes for DAT matching"). PS1/PS2/Saturn already override via
`hash_disc_container`. PSP is doubly gapped: it converts `iso → chd` but does not
even list `"chd"` in `file_extensions()` (psp.rs:33), so the produced file vanishes
from scans entirely.

**Implementation.**

1. Sega CD and Dreamcast: add, mirroring saturn.rs:364-372:

   ```rust
   fn compute_container_hashes(
       &self,
       reader: &mut dyn ReadSeek,
       algorithms: HashAlgorithms,
       file_path: Option<&Path>,
       on_progress: HashProgressFn<'_>,
   ) -> Result<Option<FileHashes>, AnalysisError> {
       hash_disc_container(reader, algorithms, file_path, "Sega CD", on_progress)
   }
   ```

   (`"Dreamcast"` respectively; `retro_junk_disc::hash::hash_disc_container` is
   already imported in the sega crate — see saturn.rs:22.)

2. PSP: add `"chd"` to `file_extensions()` and the same override with `"PSP"`.
   **Precondition to verify while implementing:** `hash_disc_container` must handle
   DVD-media CHDs (2048-byte sectors / `createdvd` output), not just CD CHDs. Read
   `retro-junk-disc/src/hash.rs::hash_disc_container` and the `chd` module; if it
   assumes CD sector layouts, extend it for the DVD case (whole-disc 2048-byte
   stream) before wiring PSP. Record findings in `formats/CHD.md`.

3. Invariant test (new, `retro-junk-lib/src/tests/` — e.g. `analyzer_invariants.rs`
   included from lib.rs like the other test modules): for every analyzer registered
   in `AnalysisContext::new()`:
   - if `chd_extensions()` contains any `Source(_)` entry → assert
     `file_extensions().contains(&"chd")` (the feature's output must be scannable);
   - if `file_extensions().contains(&"chd")` → assert
     `compute_container_hashes(&mut cursor, HashAlgorithms::Crc32Sha1, None, &|_,_|{})`
     over an empty/garbage `std::io::Cursor` is **not** `Ok(None)` — the default
     returns `Ok(None)` without touching the reader, while any real override attempts
     to parse and returns `Ok(Some(_))` or `Err(_)`. Comment the mechanism, it is
     deliberate (there is no direct way to ask "is this method overridden").

**Tests:** the invariant test *is* the test; it must fail before step 1/2 and pass
after (run it first to watch it catch Sega CD, Dreamcast, PSP).

### C3. `ChdMedia` documentation cleanup (P3) — core lib.rs:156-165

**Current.** The core doc comment ties the enum to chdman subcommands and names
consoles ("PS1, Saturn, Sega CD, PS2 CD games…"), skirting the no-console-knowledge
rule for core and binding the bottom crate to one external tool.

**Fix.** Rewrite the doc to describe physical media only:

```rust
/// Physical media class of a disc image, as needed for CHD conversion.
///
/// CD-family and DVD-family images use different CHD layouts; which class a
/// given source file belongs to is analyzer knowledge (see
/// [`RomAnalyzer::chd_extensions`]). The mapping to concrete converter
/// commands lives in the conversion layer (retro-junk-lib::chd_convert).
```

Move the `createcd`/`createdvd` mapping note to `chd_convert.rs` at the
`create_cmd` match (line 297). No rename (`ChdMedia` describes the CHD target
format, which is fine); no behavior change.

---

## Phase D — retro-junk-gui

### D1. Planning and chdman probing off the UI thread (P1) — `backend/chd_compress.rs`, `views/settings.rs`, `state.rs`

**Current.**
- `open_compress_dialog` (chd_compress.rs:38-92) runs, on the UI thread inside the
  context-menu click (console_tree.rs:244, game_table.rs:467): per-disc
  `plan_compression` (cue/gdi read+parse, `fs::metadata` per track file) **and** a
  blocking `Chdman::detect` subprocess (`Command::output`, no timeout). Hundreds of
  disc sets on a network mount freeze the UI for seconds-to-minutes; a never-exiting
  configured binary hangs the app permanently.
- The Settings probe (settings.rs:181-184) has the same blocking detect in-frame, and
  caches `Err` results keyed by path string forever (installing chdman later still
  shows "not available" until restart).

**Implementation.**

1. New message + async prompt flow:

   ```rust
   // state.rs AppMessage
   ChdCompressPromptReady { prompt: ChdCompressPrompt },
   ChdmanProbeResult { key: String, result: Result<Chdman, ChdmanUnavailable> },
   ```

   `open_compress_dialog` becomes a thin UI-thread collector: snapshot
   `(folder_name, Vec<(entry_name, Vec<PathBuf>)>)` (path clones only — no I/O), the
   `chdman_path` setting string, and the analyzer handle, then
   `spawn_background_op(app, "Preparing CHD compression…", …)` whose closure runs
   `Chdman::detect_from_setting` (B7) + `plan_batch` (B6) per entry, builds the
   `ChdCompressPrompt`, sends `ChdCompressPromptReady`, then `OperationComplete`.
   The message handler stores `app.chd_compress_prompt = Some(prompt)`. Follow the
   analyzer-into-worker pattern already used by `backend/hash.rs` (Arc-clone of the
   context / analyzer handle). Multi-disc `CompanionData` suppression disappears
   here — `plan_batch` owns that policy now.

2. Settings probe: replace the synchronous block at settings.rs:181-185 with: when
   `needs_probe && !editing && !probe_in_flight`, set a flag and
   `std::thread::spawn` a probe thread sending `ChdmanProbeResult` over
   `app.message_tx` (+ `ctx.request_repaint()`); render a spinner meanwhile. The
   handler stores `app.chdman_probe = Some((key, result))` and clears the flag.
   Add a small "Re-check" button beside the status line that clears
   `app.chdman_probe` (forcing a fresh probe), fixing the stale-`Err` cache.
   `app.chdman_probe_in_flight: bool` lives next to `chdman_probe` on the app struct.

**Tests:** egui_kittest (per project convention — never Xvfb): drive the settings
view with a stub probe result injected via the message channel; assert no
`Chdman::detect` call occurs on the UI thread (e.g. by pointing the setting at a
path in a temp dir that would fail — the frame must complete without the probe
result appearing until the message is delivered). For the prompt flow, unit-test the
handler: `ChdCompressPromptReady` sets the prompt.

### D2. Cancel + join workers on exit (P0-adjacent) — `backend/worker.rs`, `app.rs:439-454`

**Current.** `spawn_background_op` drops the `JoinHandle` (worker.rs:26-28);
`on_exit` only saves caches/settings. Closing the app mid-`delete_job_sources` kills
the process between `fs::remove_file` calls — half-deleted disc set, playlist still
referencing deleted files, stale cache persisted, orphaned chdman child writing an
unverified CHD.

**Implementation.**

1. Track handles: add `pub op_threads: std::collections::HashMap<u64, std::thread::JoinHandle<()>>`
   to `RetroJunkApp`; `spawn_background_op` inserts the handle under `op_id`.
2. `OperationComplete` handling (state.rs:2407-region): when removing the operation,
   also `remove` + `join` its handle (the thread is at/near its end; join is
   immediate).
3. `on_exit` (app.rs:439): before the existing saves —

   ```rust
   for op in &self.operations { op.cancel_token.store(true, Ordering::Relaxed); }
   for (_, handle) in self.op_threads.drain() { let _ = handle.join(); }
   self.process_pending_messages();   // apply ChdCompressComplete etc. before saving
   ```

   `process_pending_messages` = whatever fn currently drains `message_rx` in
   `update()` (factor it out if it is inline). With B2, cancellation reaches a
   running chdman within ~100 ms, so the joins are prompt; `run_chdman` already
   kills the child on cancel and B1 confines cleanup to the temp file — exit during
   compression now leaves sources intact and at worst a stale `.tmp`.

**Tests:** unit-level: after simulating an op + `on_exit`, `op_threads` is empty and
the cancel token is set. (Full exit-during-delete integration is manual; add it to
the verification checklist below.)

### D3. Guard against overlapping compressions (P1) — `state.rs`, `backend/chd_compress.rs`, menu sites

**Current.** Nothing stops a second "Compress to CHD…" while one runs
(chd_compress.rs:95 checks nothing; menus gate only on `is_scanned && entry_count > 0`,
console_tree.rs:235-241). Two workers can run chdman on the same inputs/outputs.

**Implementation.**

1. `BackgroundOperation` gains:

   ```rust
   pub kind: OperationKind,               // new enum
   /// Console folder this operation is scoped to, when applicable.
   pub scope: Option<String>,
   ```

   ```rust
   #[derive(Debug, Clone, Copy, PartialEq, Eq)]
   pub enum OperationKind { Scan, Hash, Rename, CueFix, ChdCompress, Other }
   ```

   `spawn_background_op` gains `kind: OperationKind, scope: Option<String>`
   parameters (update all existing callers — grep `spawn_background_op(`; pass
   `Other`/`None` where nothing better applies; this parameter change is shared with
   D7).
2. Helper on the app/state:

   ```rust
   pub fn chd_compress_busy(&self, folder_name: &str) -> bool {
       self.operations.iter().any(|op|
           op.kind == OperationKind::ChdCompress
           && op.scope.as_deref() == Some(folder_name))
   }
   ```

3. Gate the two menu items (console_tree.rs:243, game_table.rs:467) with
   `ui.add_enabled(!busy, egui::Button::new(...))` and a hover text
   "A CHD compression is already running for this console"; also early-return in
   `start_compression` (and in the D1 planning op) with a log line if busy — the
   menus are advisory, the check in `start_compression` is the guarantee. The
   planning op (D1) and the compression op both use `kind: ChdCompress` with the
   console's folder name as scope, so the guard covers the whole pipeline.

**Tests:** state-level: with a synthetic `ChdCompress` op for folder X in
`app.operations`, `chd_compress_busy("X")` is true, `"Y"` false; `start_compression`
no-ops when busy.

### D4. Completion handler: path-keyed, runs for every outcome, removes dangling siblings (P0/P1) — `state.rs:2351-2405`, `backend/chd_compress.rs`

**Current defects (all confirmed):**
- The handler processes only `Compressed { sources_deleted: true }` results — with
  the dialog default `delete_sources: false`, a verified compression updates
  nothing; the next rescan duplicates the game (cue entry + chd entry), and re-running
  compression hits `OutputExists` everywhere.
- Entries are found by display-name string (`find_entry_mut`, state.rs:92-96) using
  a name captured at dialog-open — stale after a rename/rescan during a long batch.
- Sibling `SingleFile` entries for cue-referenced `(Track N).bin` files (kept by the
  scanner because its dedup is stem-only, scanner.rs:251-266) are never removed; after
  deletion they dangle at deleted paths and are persisted by `save_console_cache`.

**Implementation.**

1. Carry the job in the result (state.rs:494-499):

   ```rust
   pub struct ChdCompressResult {
       pub entry_name: String,                       // display only
       pub input_name: String,                       // display only
       pub job: retro_junk_lib::chd_convert::CompressionJob,   // Clone — carries input/output/source_files
       pub outcome: ChdCompressOutcome,
   }
   ```

   Populate in the worker (backend/chd_compress.rs:202) with `item.job.clone()`.

2. Add a path-keyed lookup beside `find_entry_mut`:

   ```rust
   pub fn find_entry_by_file_mut(&mut self, file: &Path) -> Option<&mut LibraryEntry> {
       self.entries.iter_mut()
           .find(|e| e.game_entry.all_files().iter().any(|f| f == file))
   }
   ```

   The entry still holds the pre-compression path even after deletion, so matching
   `r.job.input` against `all_files()` is exact and rename-proof for this batch's
   entries. Keep `find_entry_mut` for its other callers.

3. Rework the `ChdCompressComplete` arm to process **every**
   `ChdCompressOutcome::Compressed { .. }` (drop the `sources_deleted: true` pattern
   gate):

   ```text
   for each Compressed result r:
       changed = true
       entry = console.find_entry_by_file_mut(&r.job.input)   (log a warning if None)
       if r.sources_deleted:
           match entry.game_entry:
               SingleFile(path) → *path = r.job.output
               MultiDisc → refresh via D5 (playlist now points at .chd)
           invalidate: entry.hashes = None; broken_references = None; cue_compat_issues = None
       else:
           // sources kept: entry stays on the cue/iso; hashes remain valid;
           // fingerprint invalidation below lets the next scan reconcile the new .chd
           (no per-entry mutation)
   after the loop, if any sources_deleted result existed:
       deleted: HashSet<PathBuf> = union of r.job.source_files for those results
       console.entries.retain(|e|
           !e.game_entry.all_files().iter().all(|f| deleted.contains(f))
           || e.game_entry.all_files().is_empty() == false … )
       // i.e. drop entries whose every file was deleted — this removes the
       // dangling "(Track N).bin" SingleFile ghosts. Entries updated in the loop
       // now point at the .chd, so they don't match.
   if changed: fingerprint = None; save_console_cache; recheck_invalidated_entries
   ```

4. Scanner dedup so the no-delete case doesn't duplicate on rescan
   (`scanner.rs:257`): extend the covered-extension list to include `"chd"`:

   ```rust
   if !matches!(ext.as_str(), "bin" | "img" | "iso" | "chd") { return false; }
   ```

   A `Game.chd` sharing a stem with `Game.cue` is the same game; the cue entry stays
   authoritative until sources are deleted. (Behavior note for the changelog: a
   standalone `.chd` next to a same-stem `.cue` no longer produces two library rows.)

**Tests:**
- Handler unit tests (the GUI already has message-handler tests via
  `id_stability_tests.rs`-style harnesses; otherwise test the pure helpers):
  `find_entry_by_file_mut` matches by any file of a MultiDisc entry.
- Scanner: folder `[Game.cue, Game.bin, Game.chd]` scans to a single entry
  (the cue).
- After a simulated `ChdCompressComplete` with `sources_deleted: true` for
  `Game.cue` + track bins that had their own SingleFile entries: those ghost entries
  are gone, the cue entry's path is `Game.chd`.
- With `sources_deleted: false`: entry untouched, but fingerprint cleared and cache
  saved (assert `changed == true` path taken).

### D5. `refresh_multidisc_files` must reuse the scanner's playlist logic (P1) — `state.rs:962-1001`, `scanner.rs`

**Current.** The refresh re-lists everything non-`.m3u` (state.rs:970-979, including
extensionless files) so `.sbi`/`.txt` companions become "discs"; the stem remap takes
the first match in sort order so a leftover `.bin` (delete failure) beats the new
`.chd`; the extension-only fallback (state.rs:991-996) can collapse two discs onto
one path. The scanner already solves file collection correctly
(`collect_m3u_disc_files`, scanner.rs:167-195: playlist-driven, extension-filtered
fallback with cue dedup).

**Implementation.**

1. Export the scanner helper: `pub fn collect_m3u_disc_files(dir: &Path, extensions: &HashSet<String>) -> Vec<PathBuf>`
   (drop the leading `fn`-private).
2. `refresh_multidisc_files` gains an `extensions: &HashSet<String>` parameter
   (callers have the console's platform → analyzer →
   `extension_set(analyzer.file_extensions())`; both call sites — the rename handler
   and D4 — have the console index in hand). Replace the read_dir block with:

   ```rust
   let new_files = retro_junk_lib::scanner::collect_m3u_disc_files(folder, extensions);
   if new_files.is_empty() { return; }   // don't wipe the entry on a read failure
   ```

   Because B5 rewrote the playlist before this runs, the playlist names the `.chd`s;
   leftover bins from failed deletions are not listed. `.sbi` files are excluded by
   the extension filter in the fallback path and by the playlist in the primary path.
3. Remap `disc_identifications` with claim tracking, and delete the extension-only
   fallback:

   ```rust
   let mut claimed: HashSet<&PathBuf> = HashSet::new();
   for disc in discs.iter_mut() {
       let old_stem = disc.path.file_stem().and_then(|s| s.to_str()).unwrap_or("");
       if let Some(new_path) = new_files.iter()
           .find(|p| !claimed.contains(p)
                 && p.file_stem().and_then(|s| s.to_str()).unwrap_or("") == old_stem)
       {
           claimed.insert(new_path);
           disc.path = new_path.clone();
       } else {
           log::warn!("multi-disc refresh: no new file matches disc {}", disc.path.display());
           // keep the stale path; the next full rescan reconciles
       }
   }
   ```

**Tests:** folder with playlist `[D1.chd, D2.chd]` plus stray `D1.sbi`, `D1.bin`
(failed delete): `files` = the two chds only; disc_identifications map D1→D1.chd,
D2→D2.chd; two stem-unmatched discs never collapse onto one file (claim set).

### D6. One results-dialog implementation + named status colors (P3) — `app.rs:458/663/738`, new widget

**Current.** `show_chd_compress_results_dialog` (app.rs:738) is the third
near-verbatim copy of the Window/summary/ScrollArea/colored-rows/OK scaffold
(rename at :458, cue-fix at :663), and `Color32::from_rgb(50,180,50)` /
`from_rgb(220,50,50)` literals repeat across the GUI (also settings.rs:191/196,
chd_compress_dialog.rs:31).

**Implementation.**

1. New `retro-junk-gui/src/widgets/results_dialog.rs`:

   ```rust
   pub const STATUS_OK: egui::Color32 = egui::Color32::from_rgb(50, 180, 50);
   pub const STATUS_ERR: egui::Color32 = egui::Color32::from_rgb(220, 50, 50);
   pub const STATUS_WARN: egui::Color32 = egui::Color32::from_rgb(220, 160, 40); // match existing warn color if one exists

   /// Generic modal listing per-item outcomes of a batch operation.
   /// `results` is cleared when the user dismisses the dialog.
   pub fn show_results_dialog<T>(
       ctx: &egui::Context,
       title: &str,
       results: &mut Option<Vec<T>>,
       summary: impl Fn(&[T]) -> String,
       row: impl Fn(&mut egui::Ui, &T),
   )
   ```

   Body = the common scaffold (centered `egui::Window`, `open` flag, summary label,
   `ScrollArea::vertical().max_height(300.0)` iterating `row`, OK button; dismiss on
   OK or window-close clears `*results`). Extract it from the *rename* copy (the
   oldest), then port all three call sites, passing per-op `summary` (their existing
   count lines: rename counts "already correct", CHD counts "cancelled" — keep each
   dialog's own counting in its closure) and `row` closures (their existing colored
   rows, now using the constants).
2. Replace the raw `from_rgb` literals at the sites listed above with the constants.
3. Registration: `widgets/mod.rs` module line; delete the three private fns from
   app.rs.

**Tests:** kittest smoke: open each dialog with 1-2 synthetic results, assert the
summary text renders and OK dismisses (sets the Option to None).

### D7. `ProgressDisplay` enum replaces the two bools (P3) — `state.rs:524-558`, callers

**Current.** `BackgroundOperation` carries mutually exclusive `progress_is_bytes` /
`progress_is_percent` bools; `activity_bar.rs:16-25` checks them in a fixed order;
both flags are set *post-spawn* by re-finding the op by id (chd_compress.rs:218-219,
backend/hash.rs:202).

**Implementation.**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProgressDisplay {
    #[default] Count,   // "3/10"
    Bytes,              // "234.5 MB / 4.7 GB"
    Percent,            // "42%"
}
```

Replace both bools with `pub display: ProgressDisplay`; `BackgroundOperation::new`
and `spawn_background_op` take it as a parameter (fold into the same signature
change as D3's `kind`/`scope` — one migration of all call sites). Delete the two
post-spawn `iter_mut().find(...)` mutations. `activity_bar.rs` matches the enum.

**Test:** activity-bar formatting per variant (pure function if extracted; else
kittest text assertion).

### D8. Confirmation-dialog rendering cost (P3) — `widgets/chd_compress_dialog.rs:72-93`, `state.rs`

**Current.** Every frame re-derives two `file_name().to_string_lossy()` strings and
a `format_bytes_approx` per queued item with no row virtualization — thousands of
allocations/second while a large "Compress All" dialog is open.

**Fix.** Precompute at plan time: add `pub display_line: String` to
`ChdCompressItem` (built once in the D1 planning worker:
`format!("{input_name} ({}) → {output_name}", format_bytes_approx(job.input_bytes))`),
and render with `ScrollArea::vertical().max_height(260.0).show_rows(ui, row_height, items.len(), |ui, range| …)`
emitting `ui.label(&item.display_line)`.

### D9. Save settings on focus-loss, not per keystroke (P3, pre-existing pattern) — `views/settings.rs:117/132/160`

**Current.** All three TextEdits (`metadata_dir`, `assets_dir`, `chdman_path`) set
`changed = true` on `response.changed()`, and `save_settings` writes settings.toml
that same frame → one file write per keystroke. (Pre-existing convention; the new
field copied it.)

**Fix.** For all three: `if response.lost_focus() { changed = true; }` (drop the
`|| response.changed()`). Keep the Browse-button `changed = true` (a discrete
event). Note: `lost_focus` fires on Enter and click-away, which also aligns the
save with the probe's `!editing` gate.

---

## Phase E — retro-junk-cli

### E1. Report skips instead of silence (P2) — `commands/compress.rs:82-115`

**Current.** The hand-rolled planning loop drops every `UnsupportedSource` except
counting `extension == "chd"`; an all-`.cso` PSP folder (or all-`.cdi` Dreamcast
folder) prints nothing but the final "Nothing compressed." — no reason ever shown.

**Fix.** Replace the loop (lines 82-107) with `plan_batch` (B6) over
`entries.iter().flat_map(|e| e.all_files())`. Rendering:

- keep the `already_chd` line as-is;
- print each `PlanSkip` via the existing `skips` warn-line (line 131-138) — the
  `SourceSkipClass` Display text now carries the explanation
  ("PSP cannot compress .cso files to CHD (a container format chdman cannot read)");
- the folder-skip condition (line 112) becomes
  `jobs.is_empty() && batch.skips.is_empty() && batch.already_chd == 0` — unchanged
  logic, but `skips` is now populated for unconvertible containers, so the folder
  header and reasons print.

Also apply B6's `finalize_verified` at lines 211-221 (delete+m3u block).

**Test:** golden-ish: run `plan_batch` against a temp folder of `.cso` files with the
PSP analyzer and assert the skips list is non-empty with `UnreadableContainer` class
(rendering itself is log output; the batch content is the testable unit).

### E2. Honest error variant for missing chdman (P3) — `commands/compress.rs:40`, `cli/error.rs`

**Current.** `CliError::unknown_system("chdman is required…")` renders
"Error: Unknown system: chdman is required for CHD compression".

**Fix.** In `retro-junk-cli/src/error.rs` add:

```rust
#[error("{0}")]
ExternalTool(String),
```

(plus a `pub fn external_tool(msg: impl Into<String>) -> Self` constructor if the
enum uses that convention — mirror `unknown_system`). Use it at compress.rs:40.
Leave line 75's `unknown_system` (that one genuinely concerns a platform).

### E3. Progress-message throttling + error-tail cleanup (P3) — `commands/compress.rs:193-199`, `chd_convert.rs:646-658`

1. CLI: `pb.set_message` currently allocates a formatted String on every callback
   (every chdman stderr line and every compare chunk). Track the last phase:

   ```rust
   let last_phase = std::cell::Cell::new(None::<ChdPhase>);
   let progress = |phase: ChdPhase, frac: f64| {
       pb.set_position((job_fraction(phase, frac) * 100.0) as u64);
       if last_phase.replace(Some(phase)) != Some(phase) {
           pb.set_message(match phase { /* existing three arms */ });
       }
   };
   ```

2. `run_chdman` error tail (chd_convert.rs:649-655): replace the double reverse with

   ```rust
   let start = info_lines.len().saturating_sub(4);
   let detail = info_lines[start..].join(" | ");
   ```

   (Fold into the B2 edit.)

---

## Phase F — deferred structural items (append to TODO.md, do not implement now)

Add a "CHD / analyzer-trait follow-ups" section to TODO.md with:

1. **`DiscSupport` capability object.** `RomAnalyzer` has accumulated ~5 independent
   disc-specific optional methods (`dat_source`, `redump_slug`, `dat_names`,
   `compute_container_hashes`, `chd_extensions`) whose defaults fail silently — the
   Sega CD/Dreamcast hashing gap (C2) was the proof. Proposed shape:
   `fn disc_support(&self) -> Option<&dyn DiscSupport>` returning one bundle so the
   compiler forces the whole set at once. Large cross-crate refactor; C2's invariant
   test contains the risk until then.
2. **Case-insensitive m3u entry resolution** in `find_correct_m3u_entry` (see B5
   residual gap): probe the directory case-insensitively before giving up.
3. **PREGAP-aware verification** (see A4): compensate for chdman-synthesized gaps
   instead of rejecting directive-pregap cues at planning.
4. **GDI-aware `expand_disc_set`.** `plan_compression`'s gdi branch
   (chd_convert.rs:200-214) inlines resolve-tracks-and-collect-missing that
   `disc_set::expand_disc_set` provides for cues. `DiscSetFiles` is cue-shaped
   (`cue: PathBuf` field); unifying means generalizing that struct — worth doing
   together with any future `.toc`/`.ccd` support, not before.
5. **CSO/ZSO/DAX read support for PSP** (existing TODO item) — when it lands, revisit
   the psp `chd_extensions` table (`Unconvertible` → decompress-then-convert).

---

## Verification (run after each phase; full pass at the end)

1. `cargo build && cargo test` — all workspace crates.
2. `cargo test -p retro-junk-disc -p retro-junk-lib` — the crates with new tests.
3. Manual GUI pass (or the project `/verify` flow):
   - Compress a multi-track cue/bin **without** delete-sources → library shows one
     entry; rescan does not duplicate; re-compressing reports "already exists" as a
     visible skip, and the existing CHD is untouched afterward.
   - Compress **with** delete-sources on a folder that also contains a same-stem
     second source (cue + iso) → exactly one job runs; the other is listed as a
     duplicate-output skip; the produced CHD survives.
   - Cancel mid-compression → returns within ~1 s, no `.chd`, no `.tmp`, sources
     intact.
   - Quit mid-compression (delete-sources on) → on relaunch sources are intact
     (deletion only happens post-verification, and exit now joins workers).
   - Compressed Sega CD / Dreamcast / PSP CHD identifies against Redump on rescan.
   - Settings → point chdman path at a bogus binary → UI stays responsive; Re-check
     works after installing.
4. CLI: `retro-junk compress` on a PSP folder of `.cso` files prints per-file
   "cannot compress" reasons, not a bare "Nothing compressed."

---

## How this plan keeps the codebase DRY

- **One mode→sector-size implementation** (A1): `cue::sector_size_for_mode` becomes
  the single table; `track_layout` and `hash.rs` consume it (deletes two divergent
  copies, one of them wrong).
- **One m3u-rewriting implementation** (B5): chd_convert delegates to the existing
  `RefFileFormat`/`M3uFormat` machinery instead of a weaker parallel rewriter.
- **One skip-classification policy and one batch pipeline** (B6): `plan_batch` +
  `finalize_verified` replace two divergent per-frontend loops; frontends render,
  the library decides.
- **One declarative extension table** (C1) replaces six copy-pasted trait overrides
  *and* the GUI's hardcoded probe list *and* lib's console-specific hint match.
- **One results-dialog scaffold and named status colors** (D6) replace three
  ~100-line clones and ~20 scattered `from_rgb` literals.
- **One chdman-detection entry point** (B7) replaces three trim/empty-check copies.
- **Scanner logic reused** (D5): `refresh_multidisc_files` calls
  `collect_m3u_disc_files` instead of re-implementing a worse file collector.

## How this plan maintains and improves best practices

- **Destructive operations become transactional**: temp-file publish protocol (B1),
  plan-time duplicate-output rejection (B6), overlap guard (D3), and cancel+join on
  exit (D2) together guarantee the failure path never touches files the job didn't
  create — the same discipline the repo already applies in its transactional cue/bin
  rename.
- **Fail loudly, early, and accurately**: PREGAP cues rejected at planning with the
  real reason (A4) instead of a misleading verify failure; malformed INDEX lines are
  errors (A3); CLI skips are explained (E1); missing chdman gets an honest error
  variant (E2).
- **Verification means every byte**: spans tile the whole file (A5), tracks pair by
  number (B4), and comparison is direct (B3) — no unused hash work, no
  cross-module `expect`.
- **CLAUDE.md compliance**: console-specific knowledge moves out of retro-junk-lib
  into analyzer tables (C1); the compressed-container hashing rule is enforced by a
  cross-analyzer invariant test (C2) rather than convention; new format knowledge is
  documented with sources (A6, A1 note); tests stay in separate files per the
  existing `#[path]` convention.
- **Type-driven states**: `SourceSkipClass`, `ChdExtensionRole`, `ProgressDisplay`,
  and `OperationKind` replace stringly-typed matching and mutually exclusive bools,
  making invalid states unrepresentable.
- **UI responsiveness as a rule**: no synchronous subprocess spawns or bulk I/O in
  egui `update()` (D1); background work follows the one existing worker pattern with
  lifecycle ownership (D2).
