"""Post-install step of the devzymer npm distribution.

Places the native devzymer binary inside the package and writes the platform
launchers. The binary is taken from a local build when one is available, and
otherwise downloaded from the release archive.
"""

from __future__ import annotations

import os
import platform
import shutil
import stat
import sys
import tarfile
import urllib.request
import zipfile
from pathlib import Path

PACKAGE_ROOT = Path(__file__).resolve().parent
BIN_DIR = PACKAGE_ROOT / "bin"
REPO_ROOT = PACKAGE_ROOT.parent
RELEASE_BASE = os.environ.get(
    "DEVZYMER_RELEASE_BASE",
    "https://github.com/devadeiltonlima/devzymer/releases/latest/download",
)


def binary_name() -> str:
    return "devzymer.exe" if platform.system().lower() == "windows" else "devzymer"


def archive_name() -> str:
    system = platform.system().lower()
    machine = platform.machine().lower()
    arch = "arm64" if machine in {"arm64", "aarch64"} else "x64"
    if system == "windows":
        arch = "x64"
        return f"devzymer-{arch}-pc-windows-msvc.zip"
    if system == "darwin":
        return f"devzymer-{arch}-apple-darwin.tar.gz"
    return f"devzymer-{arch}-unknown-linux-gnu.tar.gz"


def find_local_binary() -> Path | None:
    name = "devzymer.exe" if platform.system().lower() == "windows" else "devzymer"
    for profile in ("release", "debug"):
        candidate = REPO_ROOT / "target" / profile / name
        if candidate.is_file():
            return candidate
    return None


def download_binary() -> Path | None:
    url = f"{RELEASE_BASE}/{archive_name()}"
    try:
        print(f"devzymer: baixando {url}")
        with urllib.request.urlopen(url, timeout=120) as response:
            payload = response.read()
    except Exception as exc:  # noqa: BLE001 - any failure means the same thing here
        print(f"devzymer: não foi possível baixar o binário ({exc})")
        return None

    BIN_DIR.mkdir(parents=True, exist_ok=True)
    archive = BIN_DIR / archive_name()
    archive.write_bytes(payload)

    if archive.suffix == ".zip":
        with zipfile.ZipFile(archive) as zf:
            zf.extractall(BIN_DIR)
    else:
        with tarfile.open(archive) as tf:
            tf.extractall(BIN_DIR)
    archive.unlink(missing_ok=True)
    return BIN_DIR / binary_name()


def write_launchers() -> None:
    BIN_DIR.mkdir(parents=True, exist_ok=True)
    posix = BIN_DIR / "devzymer"
    posix.write_text(
        "#!/bin/sh\n"
        'DIR=$(cd "$(dirname "$0")" && pwd)\n'
        'exec "$DIR/devzymer-bin" "$@"\n',
        encoding="utf-8",
    )
    posix.chmod(posix.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    cmd = BIN_DIR / "devzymer.cmd"
    cmd.write_text(
        "@echo off\r\n"
        'setlocal\r\n'
        'set "DIR=%~dp0"\r\n'
        '"%DIR%devzymer-bin.exe" %*\r\n',
        encoding="utf-8",
    )


def main() -> int:
    source = find_local_binary()
    if source is not None:
        print(f"devzymer: usando o binário local {source}")
    else:
        source = download_binary()
        if source is None:
            print(
                "devzymer: nenhum binário nativo disponível.\n"
                "            Compile a partir do código: cargo build --release\n"
                "            ou defina DEVZYMER_RELEASE_BASE apontando para o arquivo de release.",
            )
            return 1

    BIN_DIR.mkdir(parents=True, exist_ok=True)
    target_name = "devzymer-bin.exe" if platform.system().lower() == "windows" else "devzymer-bin"
    target = BIN_DIR / target_name
    shutil.copy2(source, target)
    if platform.system().lower() != "windows":
        target.chmod(target.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH)

    write_launchers()
    print(f"devzymer: instalado em {target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
