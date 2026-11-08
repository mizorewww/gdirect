#!/usr/bin/env python3
"""Update AUR packages and optionally push them to aur.archlinux.org.

Packages maintained:
  - gdirect-bin
  - gdirect-git
  - gdirect-desktop-bin
  - gdirect-desktop-git
"""
import argparse
import os
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import time
import tomllib
import urllib.request

ROOT = Path(__file__).resolve().parents[1]
AUR_DIR = ROOT / 'packaging' / 'aur'
DEFAULT_PACKAGES = [
    'gdirect-bin',
    'gdirect-git',
    'gdirect-desktop-bin',
    'gdirect-desktop-git',
]


def get_current_version() -> str:
    cargo_toml = (ROOT / 'Cargo.toml').read_text(encoding='utf-8')
    data = tomllib.loads(cargo_toml)
    return data['package']['version']


def load_checksums(version: str, checksums_path: Path | None) -> dict[str, str]:
    if checksums_path and checksums_path.is_file():
        content = checksums_path.read_text(encoding='utf-8')
    else:
        url = f'https://github.com/mizorewww/gdirect/releases/download/v{version}/SHA256SUMS'
        content = None
        for attempt in range(6):
            try:
                with urllib.request.urlopen(url, timeout=30) as resp:
                    content = resp.read().decode('utf-8')
                    break
            except Exception as e:
                if attempt == 5:
                    raise RuntimeError(f'Failed to fetch checksums from {url} after 6 attempts: {e}') from e
                time.sleep(3)

    checksums = {}
    for line in content.splitlines():
        line = line.strip()
        if not line or line.startswith('#'):
            continue
        parts = line.split()
        if len(parts) >= 2:
            digest, filename = parts[0], parts[1].removeprefix('./')
            checksums[filename] = digest
    return checksums


def get_git_pkgver() -> str:
    try:
        out = subprocess.check_output(
            ['git', 'describe', '--long', '--tags'],
            cwd=ROOT,
            text=True,
            stderr=subprocess.DEVNULL,
        ).strip()
        # v0.12.1-19-gd9ea2b4 -> 0.12.1.r19.gd9ea2b4
        out = re.sub(r'^v', '', out)
        out = re.sub(r'([^-]*-g)', r'r\1', out)
        return out.replace('-', '.')
    except Exception:
        count = subprocess.check_output(['git', 'rev-list', '--count', 'HEAD'], cwd=ROOT, text=True).strip()
        commit = subprocess.check_output(['git', 'rev-parse', '--short=7', 'HEAD'], cwd=ROOT, text=True).strip()
        return f'r{count}.{commit}'


def update_pkgbuild(pkgname: str, version: str, checksums: dict[str, str] | None, git_pkgver: str) -> str:
    pkgbuild_path = AUR_DIR / pkgname / 'PKGBUILD'
    if not pkgbuild_path.is_file():
        raise FileNotFoundError(f'PKGBUILD for {pkgname} not found at {pkgbuild_path}')

    content = pkgbuild_path.read_text(encoding='utf-8')

    if pkgname == 'gdirect-bin':
        tar_name = f'gdirect-{version}-x86_64-unknown-linux-gnu.tar.gz'
        if not checksums or tar_name not in checksums:
            raise ValueError(f'Checksum for {tar_name} not found in SHA256SUMS')
        sha = checksums[tar_name]
        content = re.sub(r'^pkgver=.*$', f'pkgver={version}', content, flags=re.MULTILINE)
        content = re.sub(r'^pkgrel=.*$', 'pkgrel=1', content, flags=re.MULTILINE)
        content = re.sub(
            r"^sha256sums_x86_64=\('.*'\)$",
            f"sha256sums_x86_64=('{sha}')",
            content,
            flags=re.MULTILINE,
        )

    elif pkgname == 'gdirect-desktop-bin':
        tar_name = 'gdirect-desktop-linux-x64.tar.gz'
        if not checksums or tar_name not in checksums:
            raise ValueError(f'Checksum for {tar_name} not found in SHA256SUMS')
        sha = checksums[tar_name]
        content = re.sub(r'^pkgver=.*$', f'pkgver={version}', content, flags=re.MULTILINE)
        content = re.sub(r'^pkgrel=.*$', 'pkgrel=1', content, flags=re.MULTILINE)
        content = re.sub(
            r"^sha256sums_x86_64=\('.*'\)$",
            f"sha256sums_x86_64=('{sha}')",
            content,
            flags=re.MULTILINE,
        )

    elif pkgname in ('gdirect-git', 'gdirect-desktop-git'):
        content = re.sub(r'^pkgver=.*$', f'pkgver={git_pkgver}', content, flags=re.MULTILINE)

    pkgbuild_path.write_text(content, encoding='utf-8')
    return content


def push_to_aur(pkgname: str, pkgbuild_content: str, version: str) -> None:
    aur_repo_url = f'ssh://aur@aur.archlinux.org/{pkgname}.git'
    with tempfile.TemporaryDirectory() as temp_dir:
        repo_dir = Path(temp_dir) / pkgname
        print(f'==> Cloning or initializing {pkgname} from {aur_repo_url}...')

        clone_res = subprocess.run(
            ['git', 'clone', aur_repo_url, str(repo_dir)],
            capture_output=True,
            text=True,
        )

        if clone_res.returncode != 0 or not repo_dir.is_dir():
            print(f'    Repo not found or empty on AUR, creating new repo for {pkgname}...')
            repo_dir.mkdir(parents=True, exist_ok=True)
            subprocess.run(['git', 'init', '-b', 'master'], cwd=repo_dir, check=True)
            subprocess.run(['git', 'remote', 'add', 'origin', aur_repo_url], cwd=repo_dir, check=True)

        target_pkgbuild = repo_dir / 'PKGBUILD'
        target_pkgbuild.write_text(pkgbuild_content, encoding='utf-8')

        # Generate .SRCINFO using makepkg
        print('    Generating .SRCINFO...')
        try:
            srcinfo_res = subprocess.run(
                ['makepkg', '--printsrcinfo'],
                cwd=repo_dir,
                capture_output=True,
                text=True,
                check=True,
            )
            (repo_dir / '.SRCINFO').write_text(srcinfo_res.stdout, encoding='utf-8')
        except subprocess.CalledProcessError as e:
            raise RuntimeError(f'Failed to run makepkg --printsrcinfo for {pkgname}: {e.stderr}') from e

        # Ensure git identity is configured
        for key, default_val in [
            ('user.name', 'mizorewww'),
            ('user.email', 'mizorewww@users.noreply.github.com'),
        ]:
            check_key = subprocess.run(['git', 'config', key], cwd=repo_dir, capture_output=True, text=True)
            if not check_key.stdout.strip():
                subprocess.run(['git', 'config', key, default_val], cwd=repo_dir, check=True)

        subprocess.run(['git', 'add', 'PKGBUILD', '.SRCINFO'], cwd=repo_dir, check=True)
        status = subprocess.run(['git', 'status', '--porcelain'], cwd=repo_dir, capture_output=True, text=True)
        if not status.stdout.strip():
            print(f'    No changes to commit for {pkgname}.')
            return

        commit_msg = f'{pkgname}: update to {version}'
        subprocess.run(['git', 'commit', '-m', commit_msg], cwd=repo_dir, check=True)

        print(f'    Pushing {pkgname} to AUR master...')
        subprocess.run(['git', 'push', 'origin', 'master'], cwd=repo_dir, check=True)
        print(f'==> Successfully pushed {pkgname} to AUR!')


def main():
    parser = argparse.ArgumentParser(description='Update and push AUR packages')
    parser.add_argument('--version', help='Release version without v prefix (e.g. 0.12.1)')
    parser.add_argument('--checksums', type=Path, help='Path to SHA256SUMS file')
    parser.add_argument('--packages', nargs='+', choices=DEFAULT_PACKAGES, default=DEFAULT_PACKAGES,
                        help='Packages to update (default: all)')
    parser.add_argument('--push', action='store_true', help='Push changes to aur.archlinux.org')
    args = parser.parse_args()

    version = args.version or get_current_version()
    print(f'Target version: {version}')

    checksums = None
    needs_checksums = any('bin' in pkg for pkg in args.packages)
    if needs_checksums:
        print('Loading checksums...')
        checksums = load_checksums(version, args.checksums)

    git_pkgver = get_git_pkgver()
    print(f'VCS pkgver: {git_pkgver}')

    for pkgname in args.packages:
        print(f'\n--- Updating {pkgname} ---')
        content = update_pkgbuild(pkgname, version, checksums, git_pkgver)
        print(f'Updated local packaging/aur/{pkgname}/PKGBUILD')
        if args.push:
            push_to_aur(pkgname, content, version)

    print('\nAll specified AUR packages processed successfully.')


if __name__ == '__main__':
    main()
