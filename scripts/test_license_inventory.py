"""Run with python3 scripts/test_license_inventory.py; no external tools needed."""
from pathlib import Path
from tempfile import TemporaryDirectory
import license_inventory as inventory

with TemporaryDirectory() as folder:
    directory = Path(folder)
    names = ["LICENSE-MIT", "LICENCE", "NOTICE.txt", "NOTICES.md", "UNLICENSE", "OFL.txt", "COPYING", "COPYRIGHT"]
    for name in names:
        (directory / name).write_text(f"notice from {name}: © original author\n", encoding="utf-8")
    (directory / "README.md").write_text("not a notice")
    assert [p.name for p in inventory.notice_files(directory)] == sorted(names)
    inventory.collect("test", "fixture", "1", "MIT", directory)
    combined = "".join(inventory.texts)
    for name in names:
        assert f"notice from {name}: © original author" in combined
    (directory / "NOTICE-bad.txt").write_bytes(b"\xff")
    try:
        inventory.collect("test", "invalid", "1", "MIT", directory)
    except UnicodeDecodeError:
        pass
    else:
        raise AssertionError("An undecodable notice was silently discarded")
print("License notice retention check passed")
