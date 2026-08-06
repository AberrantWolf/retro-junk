#!/usr/bin/env python3
"""Strip the three catalog id keys out of archived release and carrier manifests.

Catalog work, release and media ids used to be built out of the game's title.
Correcting a title changed the id, which orphaned every manifest that named the
old one — and an archive written on one machine named rows that a differently
versioned import on another machine never created. Identity now comes from the
digests each manifest already records, so the ids are dead weight.

The Rust side stopped reading them first, and `CatalogBinding` accepts unknown
keys, so an archive that still has them keeps working. This is tidying, not a
gate: run it when convenient.

Usage:

    python3 scripts/migrate_manifests.py <archive-root>            # show what would change
    python3 scripts/migrate_manifests.py <archive-root> --apply    # change it

Dry run is the default. Running it twice changes nothing the second time.
"""

from __future__ import annotations

import argparse
import difflib
import re
import sys
import tomllib
from pathlib import Path

# The manifests that carry a [catalog_binding] table.
MANIFEST_NAMES = ("release.toml", "carrier.toml")

# The keys to remove. Everything else in the table — the source, its version,
# the DAT's name for the game, the serials, the expected track digests — is a
# description of what was matched and stays.
DEAD_KEYS = ("catalog_work_id", "catalog_release_id", "catalog_media_id")

# `[catalog_binding]`, or `[catalog_binding.something]`, at the start of a line.
TABLE_HEADER = re.compile(r"^\s*\[([^\]]+)\]\s*$")
KEY_ASSIGNMENT = re.compile(r"^\s*([A-Za-z0-9_-]+)\s*=")

# How many diffs to print before summarising the rest.
DIFFS_SHOWN = 3


def rewrite(text: str) -> str:
    """Drop the dead keys from the [catalog_binding] table, line by line.

    Deliberately not a TOML round trip: re-serialising would reorder keys,
    restyle every value and rewrite files that need no change, turning a
    reviewable diff into an unreadable one. Each line is either dropped or kept
    exactly as it was.
    """
    out = []
    in_binding = False
    for line in text.splitlines(keepends=True):
        header = TABLE_HEADER.match(line)
        if header:
            # A sub-table of catalog_binding (there are none today) is still
            # inside it; anything else ends it.
            name = header.group(1).strip()
            in_binding = name == "catalog_binding" or name.startswith("catalog_binding.")
            out.append(line)
            continue
        if in_binding:
            key = KEY_ASSIGNMENT.match(line)
            if key and key.group(1) in DEAD_KEYS:
                continue
        out.append(line)
    return "".join(out)


def manifests(root: Path) -> list[Path]:
    """Every release and carrier manifest below `root`, in a stable order.

    macOS writes an AppleDouble twin beside each file on some filesystems —
    `._carrier.toml`, holding resource-fork bytes rather than TOML. There are
    thousands of them in a real archive and every one of them would fail the
    TOML check, so they are skipped by name.
    """
    found = []
    for name in MANIFEST_NAMES:
        for path in root.rglob(name):
            if path.name.startswith("._") or any(
                part.startswith("._") for part in path.parts
            ):
                continue
            if path.is_file():
                found.append(path)
    return sorted(found)


def main() -> int:
    parser = argparse.ArgumentParser(
        description="Remove title-derived catalog ids from archive manifests."
    )
    parser.add_argument("archive_root", type=Path, help="the archive root directory")
    parser.add_argument(
        "--apply",
        action="store_true",
        help="write the changes (without this, nothing is written)",
    )
    args = parser.parse_args()

    root: Path = args.archive_root
    if not root.is_dir():
        print(f"Not a directory: {root}", file=sys.stderr)
        return 2

    paths = manifests(root)
    if not paths:
        print(f"No release.toml or carrier.toml files below {root}")
        return 0

    # Refuse to touch anything if a single file is already unreadable: a
    # partial run over a broken archive is worse than no run.
    unparseable = []
    for path in paths:
        try:
            with path.open("rb") as handle:
                tomllib.load(handle)
        except (tomllib.TOMLDecodeError, OSError) as error:
            unparseable.append((path, error))
    if unparseable:
        print(
            f"{len(unparseable)} manifest(s) are not valid TOML; nothing was changed:",
            file=sys.stderr,
        )
        for path, error in unparseable[:10]:
            print(f"  {path}: {error}", file=sys.stderr)
        return 1

    changed = []
    for path in paths:
        original = path.read_text(encoding="utf-8")
        updated = rewrite(original)
        if updated != original:
            changed.append((path, original, updated))

    print(f"Scanned {len(paths)} manifest(s) below {root}")
    if not changed:
        print("Nothing to remove — every manifest is already clean.")
        return 0

    for path, original, updated in changed[:DIFFS_SHOWN]:
        print()
        for line in difflib.unified_diff(
            original.splitlines(keepends=True),
            updated.splitlines(keepends=True),
            fromfile=str(path),
            tofile=str(path),
        ):
            print(line, end="")
    if len(changed) > DIFFS_SHOWN:
        print(f"\n… and {len(changed) - DIFFS_SHOWN} more file(s) like these.")

    if not args.apply:
        print(f"\n{len(changed)} file(s) would change. Re-run with --apply to write them.")
        return 0

    written = 0
    for path, _, updated in changed:
        # Write beside the target and rename over it, so a manifest is never
        # left half-written if this is interrupted.
        temporary = path.with_name(path.name + ".migrating")
        temporary.write_text(updated, encoding="utf-8")
        temporary.replace(path)
        written += 1
    print(f"\nRewrote {written} file(s).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
