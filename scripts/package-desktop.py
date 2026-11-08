#!/usr/bin/env python3
"""Package the desktop client as a macOS app bundle in a zip."""
import argparse
import hashlib
import plistlib
from pathlib import Path
import shutil
import subprocess
import tempfile
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
DESKTOP = ROOT / 'desktop'


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument('--target', required=True, help='aarch64-apple-darwin or x86_64-apple-darwin')
    args = parser.parse_args()
    if not args.target.endswith('-apple-darwin'):
        raise SystemExit('The desktop client is packaged for macOS only')
    version = tomllib.loads((DESKTOP / 'Cargo.toml').read_text())['package']['version']
    executable = DESKTOP / 'target' / args.target / 'release' / 'gdirect-desktop'
    if not executable.exists():
        raise SystemExit(f'Missing binary: {executable}')
    name = f'gdirect-desktop-{version}-{args.target}'
    dist = ROOT / 'dist'
    dist.mkdir(exist_ok=True)
    with tempfile.TemporaryDirectory() as temporary:
        app = Path(temporary) / 'gdirect.app'
        macos = app / 'Contents' / 'MacOS'
        resources = app / 'Contents' / 'Resources'
        macos.mkdir(parents=True)
        resources.mkdir()
        shutil.copy2(executable, macos / 'gdirect-desktop')
        shutil.copy2(DESKTOP / 'assets' / 'gdirect.icns', resources / 'gdirect.icns')
        # Redistribution notices: gdirect, the bundled fonts and OpenSSL.
        shutil.copy2(ROOT / 'LICENSE', resources / 'LICENSE')
        shutil.copy2(DESKTOP / 'assets' / 'fonts' / 'OFL.txt', resources / 'FONTS-OFL.txt')
        with urllib.request.urlopen(
            'https://raw.githubusercontent.com/openssl/openssl/openssl-3.0.0/LICENSE.txt', timeout=60
        ) as response:
            (resources / 'OPENSSL-LICENSE.txt').write_bytes(response.read())
        with (app / 'Contents' / 'Info.plist').open('wb') as plist:
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
        archive = dist / f'{name}.zip'
        archive.unlink(missing_ok=True)
        subprocess.run(['ditto', '-c', '-k', '--norsrc', '--noextattr', '--noacl', '--keepParent', str(app), str(archive)],
                       check=True)
        # Smoke test the bundled binary.
        output = subprocess.check_output([str(macos / 'gdirect-desktop'), '--version'], text=True).strip()
        assert output == f'gdirect-desktop {version}', output
    digest = hashlib.file_digest(archive.open('rb'), 'sha256').hexdigest()
    (dist / f'{archive.name}.sha256').write_text(f'{digest}  {archive.name}\n', newline='\n')
    print(archive.name)


if __name__ == '__main__':
    main()
