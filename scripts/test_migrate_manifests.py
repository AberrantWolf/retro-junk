#!/usr/bin/env python3
"""What the manifest migration must and must not do.

Run with: python3 scripts/test_migrate_manifests.py
"""

from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

from migrate_manifests import manifests, rewrite  # noqa: E402

SCRIPT = Path(__file__).resolve().parent / "migrate_manifests.py"

CARRIER = """\
schema_version = 1
carrier_id = "0198f0c1-1234-7000-8000-000000000001"
physical_copy_id = "0198f0c1-1234-7000-8000-000000000002"
serial = "SLPS-01234"
sequence_number = 1

[kind]
type = "optical_disc"

[catalog_binding]
catalog_work_id = "ps1:biohazard-3-last-escape"
catalog_release_id = "ps1:biohazard-3-last-escape:ps1:japan"
catalog_media_id = "ps1:biohazard-3-last-escape:ps1:japan:disc"
source = "redump"
dat_name = "BioHazard 3 - Last Escape (Japan)"
source_version = "2026-01-01"
serials = ["SLPS-01234"]

[[catalog_binding.expected_tracks]]
number = 1
size = 733257120
sha1 = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
"""

CLEAN = """\
schema_version = 1
carrier_id = "0198f0c1-1234-7000-8000-000000000003"
physical_copy_id = "0198f0c1-1234-7000-8000-000000000004"

[catalog_binding]
source = "no-intro"
dat_name = "Super Mario World (USA)"
"""


class RewriteTests(unittest.TestCase):
    def test_removes_only_the_three_dead_keys(self):
        result = rewrite(CARRIER)
        for dead in ("catalog_work_id", "catalog_release_id", "catalog_media_id"):
            self.assertNotIn(dead, result)
        for kept in (
            'source = "redump"',
            'dat_name = "BioHazard 3 - Last Escape (Japan)"',
            'source_version = "2026-01-01"',
            'serials = ["SLPS-01234"]',
            "size = 733257120",
        ):
            self.assertIn(kept, result)

    def test_leaves_every_other_line_byte_for_byte(self):
        result = rewrite(CARRIER)
        removed = [
            line
            for line in CARRIER.splitlines()
            if line not in result.splitlines()
        ]
        self.assertEqual(len(removed), 3, removed)

    def test_running_twice_changes_nothing_the_second_time(self):
        once = rewrite(CARRIER)
        self.assertEqual(rewrite(once), once)

    def test_an_already_clean_manifest_is_untouched(self):
        self.assertEqual(rewrite(CLEAN), CLEAN)

    def test_a_key_of_the_same_name_outside_the_binding_survives(self):
        """The table the key sits in decides, not the key's spelling."""
        text = (
            "[some_other_table]\n"
            'catalog_media_id = "keep me"\n'
            "\n"
            "[catalog_binding]\n"
            'catalog_media_id = "drop me"\n'
        )
        result = rewrite(text)
        self.assertIn('catalog_media_id = "keep me"', result)
        self.assertNotIn('catalog_media_id = "drop me"', result)


class DirectoryTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.root = Path(self.temporary.name)
        carrier_dir = self.root / "ps1" / "game" / "physical-copies" / "1" / "carriers" / "d1"
        carrier_dir.mkdir(parents=True)
        (carrier_dir / "carrier.toml").write_text(CARRIER, encoding="utf-8")
        # macOS leaves one of these beside every file on some filesystems; it
        # holds resource-fork bytes, not TOML, and must never be opened as one.
        (carrier_dir / "._carrier.toml").write_bytes(b"\x00\x05\x16\x07not toml")
        (self.root / "ps1" / "game" / "release.toml").write_text(CLEAN, encoding="utf-8")

    def tearDown(self):
        self.temporary.cleanup()

    def test_appledouble_twins_are_skipped(self):
        found = {path.name for path in manifests(self.root)}
        self.assertEqual(found, {"carrier.toml", "release.toml"})

    def run_script(self, *arguments):
        return subprocess.run(
            [sys.executable, str(SCRIPT), str(self.root), *arguments],
            capture_output=True,
            text=True,
            check=True,
        )

    def test_a_dry_run_writes_nothing(self):
        before = (self.root / "ps1/game/physical-copies/1/carriers/d1/carrier.toml").read_text()
        output = self.run_script().stdout
        after = (self.root / "ps1/game/physical-copies/1/carriers/d1/carrier.toml").read_text()
        self.assertEqual(before, after)
        self.assertIn("--apply", output)

    def test_apply_then_apply_again_is_a_no_op(self):
        self.run_script("--apply")
        path = self.root / "ps1/game/physical-copies/1/carriers/d1/carrier.toml"
        once = path.read_text()
        self.assertNotIn("catalog_media_id", once)
        second = self.run_script("--apply").stdout
        self.assertEqual(path.read_text(), once)
        self.assertIn("already clean", second)

    def test_a_broken_manifest_stops_the_whole_run(self):
        (self.root / "ps1" / "broken").mkdir()
        (self.root / "ps1" / "broken" / "release.toml").write_text(
            "this is not = = toml\n", encoding="utf-8"
        )
        result = subprocess.run(
            [sys.executable, str(SCRIPT), str(self.root), "--apply"],
            capture_output=True,
            text=True,
            check=False,
        )
        self.assertEqual(result.returncode, 1)
        carrier = self.root / "ps1/game/physical-copies/1/carriers/d1/carrier.toml"
        self.assertIn("catalog_media_id", carrier.read_text())


if __name__ == "__main__":
    unittest.main()
