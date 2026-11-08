#!/usr/bin/env python3
"""Package the desktop client for its platform.

macOS: an ad-hoc signed gdirect.app in a zip. Linux: the program with a menu
entry, icon and per-user installer in a tarball. Windows: an MSI (WiX v5).
The asset names carry no version, so links to the latest release stay put.
"""
import argparse
import hashlib
import plistlib
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DESKTOP = ROOT / 'desktop'
ARCH = {'aarch64': 'arm64', 'x86_64': 'x64'}
OPENSSL_LICENSE = 'https://raw.githubusercontent.com/openssl/openssl/openssl-3.0.0/LICENSE.txt'


def notices(folder: Path, license_name: str):
    """gdirect's license and the notices of what ships inside it."""
    shutil.copy2(ROOT / 'LICENSE', folder / license_name)
    shutil.copy2(DESKTOP / 'assets' / 'fonts' / 'OFL.txt', folder / 'FONTS-OFL.txt')
    with urllib.request.urlopen(OPENSSL_LICENSE, timeout=60) as response:
        (folder / 'OPENSSL-LICENSE.txt').write_bytes(response.read())


def check_version(executable: Path, version: str):
    output = subprocess.check_output([str(executable), '--version'], text=True).strip()
    assert output == f'gdirect-desktop {version}', output


def macos(executable: Path, version: str, archive: Path):
    with tempfile.TemporaryDirectory() as temporary:
        app = Path(temporary) / 'gdirect.app'
        contents = app / 'Contents'
        (contents / 'MacOS').mkdir(parents=True)
        (contents / 'Resources').mkdir()
        shutil.copy2(executable, contents / 'MacOS' / 'gdirect-desktop')
        shutil.copy2(DESKTOP / 'assets' / 'gdirect.icns', contents / 'Resources' / 'gdirect.icns')
        notices(contents / 'Resources', 'LICENSE')
        with (contents / 'Info.plist').open('wb') as plist:
            plistlib.dump({
                'CFBundleName': 'gdirect',
                'CFBundleDisplayName': 'gdirect',
                'CFBundleIdentifier': 'io.github.mizorewww.gdirect',
                'CFBundleExecutable': 'gdirect-desktop',
                'CFBundleIconFile': 'gdirect',
                'CFBundlePackageType': 'APPL',
                'CFBundleShortVersionString': version,
                'CFBundleVersion': version,
                'CFBundleInfoDictionaryVersion': '6.0',
                'LSMinimumSystemVersion': '15.0',
                'LSApplicationCategoryType': 'public.app-category.productivity',
                'NSHighResolutionCapable': True,
                'NSHumanReadableCopyright': 'GPL-2.0-or-later',
            }, plist)
        # Not notarized: an ad-hoc signature keeps the bundle intact on Apple
        # silicon; users confirm the first launch in System Settings.
        subprocess.run(['xattr', '-cr', str(app)], check=True)
        subprocess.run(['codesign', '--force', '--deep', '--sign', '-', str(app)], check=True)
        subprocess.run(['codesign', '--verify', '--deep', '--strict', str(app)], check=True)
        subprocess.run(['ditto', '-c', '-k', '--norsrc', '--noextattr', '--noacl', '--keepParent',
                        str(app), str(archive)], check=True)
        check_version(contents / 'MacOS' / 'gdirect-desktop', version)


def linux(executable: Path, version: str, archive: Path):
    with tempfile.TemporaryDirectory() as temporary:
        folder = Path(temporary) / 'gdirect-desktop'
        folder.mkdir()
        shutil.copy2(executable, folder / 'gdirect-desktop')
        shutil.copy2(DESKTOP / 'assets' / 'gdirect.png', folder / 'gdirect.png')
        for name in ['gdirect.desktop', 'install.sh']:
            shutil.copy2(DESKTOP / 'packaging' / name, folder / name)
        notices(folder, 'LICENSE')
        check_version(folder / 'gdirect-desktop', version)
        with tarfile.open(archive, 'w:gz') as output:
            output.add(folder, arcname='gdirect-desktop')


def windows(executable: Path, version: str, archive: Path):
    check_version(executable, version)
    with tempfile.TemporaryDirectory() as temporary:
        bundle = Path(temporary)
        shutil.copy2(executable, bundle / 'gdirect-desktop.exe')
        shutil.copy2(DESKTOP / 'assets' / 'gdirect.ico', bundle / 'gdirect.ico')
        notices(bundle, 'LICENSE.txt')
        subprocess.run(['wix', 'build', str(DESKTOP / 'packaging' / 'gdirect.wxs'), '-arch', 'x64',
                        '-d', f'Version={version}', '-d', f'Bundle={bundle}', '-o', str(archive)],
                       check=True)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', required=True,
                        help='aarch64-apple-darwin, x86_64-apple-darwin, x86_64-unknown-linux-gnu or x86_64-pc-windows-gnu')
    args = parser.parse_args()
    version = tomllib.loads((DESKTOP / 'Cargo.toml').read_text())['package']['version']
    arch = ARCH[args.target.split('-')[0]]
    dist = ROOT / 'dist'
    dist.mkdir(exist_ok=True)
    release = DESKTOP / 'target' / args.target / 'release'
    if args.target.endswith('-apple-darwin'):
        archive, package, binary = dist / f'gdirect-desktop-macos-{arch}.zip', macos, 'gdirect-desktop'
    elif args.target.endswith('-linux-gnu'):
        archive, package, binary = dist / f'gdirect-desktop-linux-{arch}.tar.gz', linux, 'gdirect-desktop'
    elif args.target.endswith('-windows-gnu'):
        archive, package, binary = dist / f'gdirect-desktop-windows-{arch}.msi', windows, 'gdirect-desktop.exe'
    else:
        raise SystemExit(f'Unsupported target: {args.target}')
    executable = release / binary
    if not executable.exists():
        raise SystemExit(f'Missing binary: {executable}')
    archive.unlink(missing_ok=True)
    package(executable, version, archive)
    digest = hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest()
    (dist / f'{archive.name}.sha256').write_text(f'{digest}  {archive.name}\n', newline='\n')
    print(archive.name)


if __name__ == '__main__':
    main()
