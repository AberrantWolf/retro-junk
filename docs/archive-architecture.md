# Preservation archive and playable library

Version 0.4 separates two jobs that cannot be represented honestly by one file:

- the **archive** stores preservation masters, physical-copy provenance, photographs, scraped source files, and append-only evidence;
- the **playable library** stores replaceable representations selected for emulator compatibility, such as CHD;
- the **workspace** is disposable scratch space for tools such as redumper and chdman.

Device-local paths live in a collection profile. The archive itself is portable and contains no absolute paths. SQLite is a rebuildable search/index projection; TOML manifests and JSON evidence in the archive are authoritative.

The profile workspace defaults to a device-local cache directory rather than beneath the archive. Imports copy each source package into a disposable workspace lease with one sequential source read while calculating CRC32, MD5, SHA-1, and SHA-256. Before reading source bytes on Unix hosts, staging verifies that the workspace has room for the complete package plus a 64 MiB safety reserve. Identification, normalized ROM hashing, CUE track hashing, and Redumper inspection use that local package. Archive publication then compares its copy pass with the staged digests and re-reads the archive destination with SHA-256 before publishing it. This deliberately trades temporary local space for fewer network reads; `--consume` retains an additional source verification before deletion.

## Layout

```text
archive/
  retro-junk-archive.toml
  <platform>/
    <release>/
      release.toml
      artwork/                # authoritative downloaded originals
      videos/
      documents/
      metadata/
      physical-copies/
        copy-01/              # stable, human-readable ordinal; UUID is in manifest
          physical-copy.toml  # ownership, condition, acquisition, provenance
          photos/
          provenance/
          documents/
          carriers/
            <serial-or-carrier>/
              carrier.toml
              dumps/
                <date>-<uuid>/
                  dump.toml
                  raw/        # immutable preservation-master bytes
                  intermediates/ # optional retained canonical CUE/BIN or ISO
                  evidence/   # append-only verification/build records
```

Release, physical-copy, carrier, dump, representation, verification, and build identities are UUIDv7 values. A release can contain multiple owned physical copies; one physical copy can contain multiple carriers, such as the discs in a multi-disc game. `copy-01` is a stable display path, while `physical_copy_id` is the durable identity used by manifests and the database.

The vocabulary is deliberately narrow: a **release** is the cataloged edition, a **physical copy** is an owned specimen, a **carrier** is a cartridge/disc/card/tape within it, a **dump** is one capture event, and a **representation** is either the preservation master or a derived playable form. “Media” remains a catalog/frontend term only; “supporting files” covers artwork, video, documentation, photographs, provenance, and metadata without making those unlike things one archive object.

## Evidence semantics

- **Present** means the expected regular files and sizes exist on this device; it is not a validity claim.
- **Integrity verified** means archived bytes still have the SHA-256 recorded at ingest and no unrecorded files appeared in `raw/`.
- **Reproduced** means redumper could regenerate a track set from the raw master.
- **Catalog verified** means the complete ordered set of regenerated track sizes and hashes matched one catalog medium.
- **Round-trip verified** means a playable derivative was decoded and compared with its canonical input.

A `.scram` hash is an integrity hash, not a Redump identity hash. Redumper audits copy the raw set to a unique workspace, run `split` and `hash` there, parse the emitted `<rom .../>` records with the same Logiqx parser used for Redump DATs, and then delete generated BIN/CUE files. Failed splits are recorded as evidence and do not condemn the archived source.

CHD and RVZ builds require current catalog evidence by default. `--allow-unverified` permits an explicitly warned build; its evidence records that it was not catalog verified. chdman and DolphinTool derivatives are converted back and compared with their canonical inputs in either case. Evidence is current only while its `input_manifest_sha256` matches the representation it describes.

## CLI workflow

```console
retro-junk archive init /collections/archive --name "My Collection"

# Auto-discover packages directly below this directory (or below platform folders),
# identify them by complete hashes, file hashes, header serial, or folder serial,
# and copy every ready package into its catalog-derived archive location.
retro-junk archive import /incoming/dumps \
  --archive-root /collections/archive --dry-run

retro-junk archive import /incoming/dumps \
  --archive-root /collections/archive --yes

# Optional move-like mode. Each source is removed only after the archive copy
# or an exact existing package has been rehashed successfully.
retro-junk archive import /incoming/dumps \
  --archive-root /collections/archive --consume --yes

# Promote loose cartridge ROMs from an existing playable tree. Existing files
# remain in place and become adopted, byte-identical playable representations.
retro-junk archive import-playable /collections/roms \
  --archive-root /collections/archive --dry-run

retro-junk archive import-playable /collections/roms \
  --archive-root /collections/archive --yes

# Manual escape hatch when catalog identification is unavailable.
retro-junk archive ingest /incoming/disc-dump \
  --archive-root /collections/archive \
  --platform psx --title "Example Game" --region usa \
  --serial SLUS-00000 --sequence-number 1 --format redumper-raw

retro-junk archive add-release-file /downloads/box-front.png \
  --archive-root /collections/archive --release-id <uuid> \
  --category artwork --asset-type box-front --source screenscraper --source-url <url>

retro-junk archive add-physical-copy-file /photos/cartridge-front.jpg \
  --archive-root /collections/archive --physical-copy-id <uuid> \
  --category photo --asset-type physical-copy-front

retro-junk archive verify /collections/archive

# CRC32/MD5/SHA-1 verification for single-file cartridge/ISO masters.
retro-junk archive verify-catalog /collections/archive

retro-junk archive audit-redumper /collections/archive \
  --redumper /usr/local/bin/redumper

# Persist conversion intent independently of the device doing the build.
retro-junk archive policy /collections/archive \
  --carrier-id <uuid> --format chd

# Or define the inherited default for a whole platform.
retro-junk archive policy-default /collections/archive \
  --platform psx --format chd

retro-junk archive build-chd /collections/archive \
  --playable-root /collections/roms --dump-id <uuid>

# Execute all effective policies. Re-running skips current, present outputs.
retro-junk archive build /collections/archive \
  --playable-root /collections/roms

# GameCube/Wii ISO masters can be policy-built to round-trip-verified RVZ.
retro-junk archive build-rvz /collections/archive \
  --playable-root /collections/roms --dump-id <uuid>

# Cartridge and other single-file masters are byte-identical projections.
retro-junk archive mirror /collections/archive \
  --playable-root /collections/roms --dump-id <uuid>

retro-junk archive reindex /collections/archive \
  --playable-root /collections/roms

# Adopt byte-identical legacy playable files and inventory everything else.
retro-junk archive adopt-playable /collections/archive \
  --playable-root /collections/roms

# Rebuild frontend media projections from archived originals.
retro-junk archive project-frontend-files /collections/archive \
  --media-root /collections/roms-media

# Quarantine abandoned staging/work directories without deleting them.
retro-junk archive recover /collections/archive
```

Import records CRC32, MD5, SHA-1, and SHA-256 while copying into an owned sibling staging directory, re-reads the staged bytes, and publishes by atomic rename. Original raw filenames remain unchanged as evidence; catalog identification automatically supplies the normalized release, physical-copy, and carrier directory names. Sources are retained unless `--consume` is explicit. Integrity and audit commands append new JSON evidence rather than overwriting history. Mutating CLI/GUI operations take a recoverable process lock, and manifest/evidence writes are synced before publication. Reindexing can reconstruct all archive projection rows after database loss.

For cartridge ROMs, catalog verification hashes the logical payload through the registered platform analyzer. This strips catalog-irrelevant headers such as iNES and SNES copier headers and normalizes N64 byte order for matching, while the archive retains the exact input file. `import-playable` accepts loose supported ROM files beneath platform directories; compressed ZIP/7z libraries must currently be unpacked first.

Catalog platform identity and physical platform identity are deliberately separate. A combined catalog remains the verification namespace, while archival releases retain the name of the physical hardware variant. The initial mappings are NES/Famicom, SNES/Super Famicom, Genesis/Mega Drive, and PC Engine/TurboGrafx-16. An explicit platform hint wins, then a recognized source-folder name, then an unambiguous catalog region. Reindexing performs a one-time upgrade of pre-release 0.4 archives by moving affected release directories and updating their release manifests without copying dump payloads. A catalog binding still records the combined catalog platform, so the physical distinction does not weaken checksum evidence or playable-library matching.

## GUI model

The Collection view is release-centric and rolls up physical copies, carriers, preservation masters, playable derivatives, and each evidence class. Its **Import dumps…** button opens a modal workflow that inventories and identifies a selected directory in the background, presents ambiguous catalog or physical-copy choices, and blocks the main UI only while that workflow is open. **Import existing playable library…** uses the same review workflow but disables source removal and records retained matching files as existing playable representations. It can also ingest a new dump for an existing carrier manually, refresh/reindex, verify stored bytes, edit condition/acquisition/provenance, attach physical photos/documents, and set a carrier policy. Profiles pair archive/playable/workspace roots and can initialize the portable archive from Settings.

The Library view combines playable filesystem entries with the archive projection without pretending that an archived-only carrier has a playable path. Playable rows show whether they are unarchived, archived in the preferred format, or archived with a non-preferred playable format. A separate per-console queue lists archival carriers whose preferred playable representation is absent. The toolbar writes the console's default policy to the portable root manifest. That small edit incrementally updates only inherited policy rows for the affected profile/platform in SQLite; explicit carrier overrides remain unchanged, and a full network archive traversal is reserved for reindex and recovery. Queue actions mirror single-file archival-equivalent ROMs with byte verification or build CHDs with round-trip verification, append build evidence, refresh the archive projection, and rescan the playable library. Disc inputs prepared from Redumper raw sets use the device-local profile workspace; unavailable source/target conversions remain visible but disabled rather than silently changing the requested policy.

The preferred-format selectors are platform-aware rather than global lists. They offer native emulator files for cartridge-oriented systems; CHD and BIN/CUE for PS1, Sega CD, Saturn, and Dreamcast; CHD, ISO, and BIN/CUE for PS2; CHD and ISO for PSP; RVZ and ISO for GameCube/Wii; and ISO for Xbox 360. When no modeled representation honestly matches a mainstream emulator's input model—currently original Xbox, PS3, and Vita—only **No preference** is offered. A previously stored unsupported value remains visible and labeled unsupported until the user changes it. Original Xbox is deliberately excluded because xemu's XISO is not equivalent to a Redump-style ISO and needs its own representation and conversion pipeline.

Startup blocks interaction only when the durable catalog must be moved, validated, created from an existing legacy database, or schema-migrated. With a current local database, the last committed archive projection is immediately usable while a tracked background refresh locks and scans the authoritative archive. SQLite WAL transactions keep readers on the prior complete projection until the replacement commits atomically; archive-scoped GUI mutations are disabled or rejected by the same archive lock while refresh is active. Saved network-root probing is also independent of database readiness.

Passing `--archive-root` to the existing `scrape` command adopts downloaded ScreenScraper originals into the matched release. `archive project-frontend-files` recreates frontend layout files from those originals. Physical photographs and provenance remain attached to physical copies, not catalog releases.

## Device-local Inbox and selective storage

Representation manifests describe what is known; `presence_state` describes what exists on the current device. Missing playable derivatives are normal on a selectively synced device and remain rebuildable. `adopt-playable` never moves or rewrites an existing library: byte-identical matches become build evidence, while catalog-only, ambiguous, and unknown files are written to `.retro-junk/playable-inbox.toml` for review.
