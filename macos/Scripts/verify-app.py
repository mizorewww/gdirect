#!/usr/bin/env python3

import argparse
import os
from pathlib import Path
import plistlib
import subprocess
import sys


class VerificationError(Exception):
    pass


def require(condition, message):
    if not condition:
        raise VerificationError(message)


def architectures(executable):
    result = subprocess.run(
        ["lipo", "-archs", str(executable)],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    require(
        result.returncode == 0,
        f"{executable.name} is not a readable Mach-O executable: {result.stderr.strip()}",
    )
    return set(result.stdout.split())


def verify(app, expected_version, expected_arch):
    require(app.is_dir(), f"APP bundle is missing: {app}")
    contents = app / "Contents"
    plist_path = contents / "Info.plist"
    require(plist_path.is_file(), "Contents/Info.plist is missing")
    with plist_path.open("rb") as stream:
        plist = plistlib.load(stream)
    require(plist.get("CFBundleExecutable") == "GDirectApp", "CFBundleExecutable must be GDirectApp")
    require(plist.get("CFBundleName") == "GDirect", "CFBundleName must be GDirect")
    require(plist.get("CFBundleDisplayName") == "GDirect", "CFBundleDisplayName must be GDirect")
    require(plist.get("CFBundlePackageType") == "APPL", "CFBundlePackageType must be APPL")

    executables = [contents / "MacOS" / "GDirectApp", contents / "MacOS" / "gdirect"]
    for executable in executables:
        require(executable.is_file(), f"Contents/MacOS/{executable.name} is missing")
        require(os.access(executable, os.X_OK), f"{executable.name} is not executable")
        arches = architectures(executable)
        require(
            expected_arch in arches,
            f"{executable.name} architecture {sorted(arches)} does not include {expected_arch}",
        )

    ca = contents / "MacOS" / "cacert.pem"
    require(ca.is_file() and ca.stat().st_size > 0, "Contents/MacOS/cacert.pem is missing or empty")
    require("-----BEGIN CERTIFICATE-----" in ca.read_text(), "cacert.pem is not a PEM CA bundle")

    result = subprocess.run(
        [str(executables[1]), "--version"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    require(result.returncode == 0, f"gdirect --version failed: {result.stdout.strip()}")
    require(
        result.stdout.strip() == f"gdirect {expected_version}",
        f"CLI version mismatch: expected gdirect {expected_version}, got {result.stdout.strip()}",
    )

    lint = subprocess.run(
        ["plutil", "-lint", str(plist_path)],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
    )
    require(lint.returncode == 0, f"Info.plist is invalid: {lint.stdout.strip()}")
    for target in (app, *executables):
        signature = subprocess.run(
            ["codesign", "--verify", "--strict", str(target)],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        require(
            signature.returncode == 0,
            f"Code signature verification failed for {target.name}: {signature.stdout.strip()}",
        )


def main():
    parser = argparse.ArgumentParser(description="Verify a packaged GDirect.app")
    parser.add_argument("app", type=Path)
    parser.add_argument("--version", required=True)
    parser.add_argument("--arch", required=True, choices=("arm64", "x86_64"))
    args = parser.parse_args()
    try:
        verify(args.app, args.version, args.arch)
    except (VerificationError, OSError, plistlib.InvalidFileException) as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    print(f"Verified {args.app}: GDirectApp, gdirect {args.version}, {args.arch}, CA, plist, signature")
    return 0


if __name__ == "__main__":
    sys.exit(main())
