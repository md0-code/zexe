"""Create the unpacked distribution and its native release archive."""
import json
import argparse
from pathlib import Path
import platform
import re
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def dependency_notices(payload):
    host = subprocess.check_output(["rustc", "-vV"], text=True)
    target = next(line.split(": ", 1)[1] for line in host.splitlines() if line.startswith("host: "))
    metadata = subprocess.check_output(
        ["cargo", "metadata", "--locked", "--offline", "--format-version", "1", "--filter-platform", target],
        cwd=ROOT, encoding="utf-8")
    directory = payload / "docs/licenses"
    directory.mkdir()
    with (directory / "dependencies.txt").open("w", encoding="utf-8") as index:
        for crate in json.loads(metadata)["packages"]:
            if crate["source"] is None:
                continue
            name = crate["name"] + "-" + crate["version"]
            print(name, crate["license"] or crate["license_file"], file=index)
            source = Path(crate["manifest_path"]).parent
            notices = [p for p in source.iterdir() if p.is_file() and
                       p.name.upper().startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "COPYRIGHT"))]
            if crate["license_file"]:
                notices.append(source / crate["license_file"])
            for notice in notices:
                destination = directory / name
                destination.mkdir(exist_ok=True)
                shutil.copy2(notice, destination / notice.name)


def package():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.parse_args()
    version = (ROOT / "src/VERSION").read_text().strip()
    os_name = platform.system().lower()
    machine = platform.machine().lower()
    arch = {"amd64": "x64", "x86_64": "x64", "aarch64": "arm64"}.get(machine, machine)
    name = f"zexe-{version}-{os_name}-{arch}"
    dist = ROOT / "dist"
    releases = dist / "releases"
    releases.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(dir=dist) as temporary:
        payload = Path(temporary) / name
        payload.mkdir()
        subprocess.run(["cargo", "build", "--release", "--locked"], cwd=ROOT, check=True)
        suffix = ".exe" if os_name == "windows" else ""
        for binary in ("zexe-runner", "zexe-bundler"):
            shutil.copy2(ROOT / "build/release" / (binary + suffix), payload)
        shutil.copy2(ROOT / "README.md", payload)
        shutil.copytree(ROOT / "docs", payload / "docs",
                        ignore=shutil.ignore_patterns("DEVELOPMENT.md", "dev", "__pycache__"))
        dependency_notices(payload)
        if os_name == "windows":
            archive = Path(temporary) / f"{name}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED, strict_timestamps=False) as out:
                for path in sorted(payload.rglob("*")):
                    if path.is_file():
                        out.write(path, path.relative_to(payload.parent))
        else:
            archive = Path(temporary) / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as out:
                out.add(payload, arcname=name)
        shutil.copytree(payload, dist, dirs_exist_ok=True)
        archive.replace(releases / archive.name)
        # Delete only older release archives owned by this project.
        for previous in releases.iterdir():
            match = re.fullmatch(r"zexe-(\d+\.\d+\.\d+)-[A-Za-z0-9_]+-[A-Za-z0-9_]+\.(?:zip|tar\.gz)", previous.name)
            if match and match[1] != version:
                previous.unlink()
        print(releases / archive.name)


if __name__ == "__main__":
    package()
