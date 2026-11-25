"""Create the unpacked distribution and its native release archive."""
import argparse
from pathlib import Path
import platform
import shutil
import subprocess
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


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
        if os_name == "windows":
            archive = Path(temporary) / f"{name}.zip"
            with zipfile.ZipFile(archive, "w", compression=zipfile.ZIP_DEFLATED) as out:
                for path in sorted(payload.rglob("*")):
                    if path.is_file():
                        out.write(path, path.relative_to(payload.parent))
        else:
            archive = Path(temporary) / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as out:
                out.add(payload, arcname=name)
        shutil.copytree(payload, dist, dirs_exist_ok=True)
        archive.replace(releases / archive.name)
        print(releases / archive.name)


if __name__ == "__main__":
    package()
