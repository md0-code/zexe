"""Show, check or bump the release version stored in src/VERSION."""
import argparse
from pathlib import Path
import re

ROOT = Path(__file__).resolve().parents[1]
VERSION = ROOT / "src/VERSION"


def synchronize(old, new):
    manifest = ROOT / "Cargo.toml"
    data = manifest.read_text(encoding="utf-8")
    field = f'version = "{old}"'
    if field not in data:
        raise SystemExit("Cargo workspace version differs from src/VERSION")
    if old != new:
        import subprocess
        manifest.write_text(data.replace(field, f'version = "{new}"'), encoding="utf-8", newline="\n")
        try:
            subprocess.run(["cargo", "update", "--workspace", "--offline"], cwd=ROOT, check=True)
        except (OSError, subprocess.CalledProcessError):
            manifest.write_text(data, encoding="utf-8", newline="\n")
            raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="action", required=True)
    commands.add_parser("show")
    commands.add_parser("check")
    commands.add_parser("bump").add_argument("part", choices=("major", "minor", "patch"))
    args = parser.parse_args()
    old = VERSION.read_text(encoding="ascii").strip()
    if not re.fullmatch(r"(?:0|[1-9][0-9]*)(?:\.(?:0|[1-9][0-9]*)){2}", old):
        parser.error(f"invalid release version {old!r}")
    if args.action == "bump":
        parts = list(map(int, old.split(".")))
        index = ("major", "minor", "patch").index(args.part)
        parts[index] += 1
        for lower in range(index + 1, 3):
            parts[lower] = 0
        new = ".".join(map(str, parts))
        synchronize(old, new)
        VERSION.write_text(new + "\n", encoding="ascii", newline="\n")
        print(new)
    else:
        synchronize(old, old)
        print(old)


if __name__ == "__main__":
    main()
