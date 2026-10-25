import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tarfile
import tempfile
import unittest


REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_MACOS = REPO_ROOT / "macos"


class PackagingTests(unittest.TestCase):
    def setUp(self):
        self.temp_dir = tempfile.TemporaryDirectory(prefix="gdirect-packaging-tests-")
        self.root = Path(self.temp_dir.name)
        self.macos = self.root / "macos"
        self.macos.mkdir()
        for relative in (
            "build-app.sh",
            "Resources/Info.plist",
            "Resources/CLIRelease.json",
            "Scripts/verify-app.py",
        ):
            source = SOURCE_MACOS / relative
            if source.exists():
                destination = self.macos / relative
                destination.parent.mkdir(parents=True, exist_ok=True)
                shutil.copy2(source, destination)
        (self.macos / "Package.swift").write_text("// fake package\n")
        self.fake_bin = self.root / "bin"
        self.fake_bin.mkdir()
        self.scratch = self.root / "scratch"
        self.scratch.mkdir()
        self._make_fake_tools()
        self.archive = self._make_archive()
        self._write_release(self.archive)

    def tearDown(self):
        self.temp_dir.cleanup()

    def _write_executable(self, name, contents):
        path = self.fake_bin / name
        path.write_text(contents)
        path.chmod(0o755)

    def _make_fake_tools(self):
        self._write_executable(
            "uname",
            "#!/bin/sh\nprintf '%s\\n' \"${FAKE_UNAME_ARCH:-arm64}\"\n",
        )
        self._write_executable(
            "swift",
            """#!/bin/bash
set -euo pipefail
if [[ " $* " == *" test "* ]]; then
  exit 0
fi
scratch=
while [[ $# -gt 0 ]]; do
  if [[ "$1" == "--scratch-path" ]]; then
    scratch="$2"
    shift 2
  else
    shift
  fi
done
mkdir -p "$scratch/release"
cat > "$scratch/release/GDirectApp" <<'EOF'
#!/bin/sh
# ARCHS=arm64
exit 0
EOF
chmod +x "$scratch/release/GDirectApp"
""",
        )
        self._write_executable(
            "lipo",
            """#!/bin/bash
set -euo pipefail
if [[ "${1:-}" != "-archs" ]]; then
  exit 2
fi
while IFS= read -r line; do
  case "$line" in
    "# ARCHS="*)
      printf '%s\\n' "${line#"# ARCHS="}"
      exit 0
      ;;
  esac
done < "$2"
exit 1
""",
        )
        self._write_executable(
            "codesign",
            """#!/bin/bash
set -euo pipefail
if [[ " $* " == *" --verify "* && " $* " == *" --deep "* ]]; then
  printf '%s\\n' 'cacert.pem is a resource, not a signed code subcomponent' >&2
  exit 1
fi
target="${!#}"
if [[ "${FAKE_FAIL_DIST_VERIFY:-}" == "1" && " $* " == *" --verify "* && "$target" == */macos/dist/* && -d "$target/Contents" ]]; then
  printf '%s\\n' 'injected dist verification failure' >&2
  exit 1
fi
if [[ " $* " == *" --sign "* && "$target" == *.app && -f "$target/Contents/MacOS/cacert.pem" && ! -L "$target/Contents/MacOS/cacert.pem" ]]; then
  printf '%s\\n' 'a regular cacert.pem under Contents/MacOS cannot be sealed as code' >&2
  exit 1
fi
if [[ " $* " == *" --verify "* && ( "$target" == *.app || "$target" == */macos/dist/.GDirect.app.tmp.* ) ]]; then
  marker="$GDIRECT_BUILD_SCRATCH_PARENT/dist-metadata-probe"
  if [[ "$target" == */macos/dist/* && -d "$target/Contents" && ! -e "$marker" ]]; then
    touch "$marker"
    printf '%s\\n' 'transient post-copy metadata injection' >&2
    exit 1
  fi
  attrs="$(/usr/bin/xattr "$target")"
  case "$attrs" in
    *com.apple.FinderInfo*|*com.apple.ResourceFork*|*com.apple.fileprovider*)
      printf 'resource fork detritus on %s: %s\\n' "$target" "$attrs" >&2
      exit 1
      ;;
  esac
fi
exit 0
""",
        )
        self._write_executable(
            "ditto",
            """#!/bin/bash
set -euo pipefail
/usr/bin/ditto "$@"
target="${!#}"
/usr/bin/xattr -wx com.apple.FinderInfo 0000000000000000000000000000000000000000000000000000000000000000 "$target"
""",
        )
        self._write_executable(
            "curl",
            """#!/bin/bash
set -euo pipefail
output=
url=
while [[ $# -gt 0 ]]; do
  case "$1" in
    -o|--output)
      output="$2"
      shift 2
      ;;
    -*)
      shift
      ;;
    *)
      url="$1"
      shift
      ;;
  esac
done
cp "$FIXTURE_ARCHIVE" "$output"
""",
        )

    def _make_archive(self, version="1.2.3", reported_version=None, include_ca=True):
        reported_version = reported_version or version
        asset = f"gdirect-{version}-aarch64-apple-darwin.tar.gz"
        source = self.root / "archive-source" / asset.removesuffix(".tar.gz")
        if source.exists():
            shutil.rmtree(source)
        source.mkdir(parents=True)
        cli = source / "gdirect"
        cli.write_text(
            "#!/bin/sh\n"
            "# ARCHS=arm64\n"
            "if [ \"${1:-}\" = \"--version\" ]; then\n"
            f"  printf '%s\\n' 'gdirect {reported_version}'\n"
            "  exit 0\n"
            "fi\n"
            "exit 0\n"
        )
        cli.chmod(0o755)
        if include_ca:
            (source / "cacert.pem").write_text(
                "-----BEGIN CERTIFICATE-----\nfixture\n-----END CERTIFICATE-----\n"
            )
        archive = self.root / asset
        with tarfile.open(archive, "w:gz") as bundle:
            bundle.add(source, arcname=source.name)
        return archive

    def _write_release(self, archive, sha256=None):
        digest = sha256 or hashlib.sha256(archive.read_bytes()).hexdigest()
        release = {
            "version": "1.2.3",
            "release_url": "https://example.test/releases/download/v1.2.3",
            "archives": {
                "arm64": {"asset": archive.name, "sha256": digest},
                "x86_64": {
                    "asset": "gdirect-1.2.3-x86_64-apple-darwin.tar.gz",
                    "sha256": "0" * 64,
                },
            },
        }
        resources = self.macos / "Resources"
        resources.mkdir(exist_ok=True)
        (resources / "CLIRelease.json").write_text(json.dumps(release))

    def _run_build(self, *args, extra_env=None):
        script = self.macos / "build-app.sh"
        self.assertTrue(script.exists(), "macos/build-app.sh must exist")
        env = os.environ.copy()
        env.update(
            {
                "PATH": f"{self.fake_bin}{os.pathsep}{env['PATH']}",
                "FIXTURE_ARCHIVE": str(self.archive),
                "GDIRECT_BUILD_SCRATCH_PARENT": str(self.scratch),
            }
        )
        if extra_env:
            env.update(extra_env)
        return subprocess.run(
            [str(script), *args],
            cwd=self.root,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
            timeout=30,
        )

    def _make_local_cli(self, version="1.2.3"):
        directory = self.root / "local-cli"
        directory.mkdir(exist_ok=True)
        cli = directory / "gdirect"
        cli.write_text(
            "#!/bin/sh\n"
            "# ARCHS=arm64\n"
            f"printf '%s\\n' 'gdirect {version}'\n"
        )
        cli.chmod(0o755)
        (directory / "cacert.pem").write_text(
            "-----BEGIN CERTIFICATE-----\nlocal\n-----END CERTIFICATE-----\n"
        )
        return cli

    def test_download_build_produces_complete_verified_bundle(self):
        result = self._run_build()
        self.assertEqual(result.returncode, 0, result.stdout)
        contents = self.macos / "dist" / "GDirect.app" / "Contents"
        self.assertTrue((contents / "MacOS" / "GDirectApp").is_file())
        self.assertTrue((contents / "MacOS" / "gdirect").is_file())
        self.assertTrue((contents / "MacOS" / "cacert.pem").is_file())
        self.assertTrue((contents / "Info.plist").is_file())

    def test_unsupported_architecture_fails_before_build(self):
        result = self._run_build(extra_env={"FAKE_UNAME_ARCH": "ppc64"})
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unsupported architecture", result.stdout)
        self.assertFalse((self.macos / "dist" / "GDirect.app").exists())

    def test_checksum_mismatch_is_rejected(self):
        self._write_release(self.archive, sha256="f" * 64)
        result = self._run_build()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Checksum mismatch", result.stdout)
        self.assertFalse((self.macos / "dist" / "GDirect.app").exists())

    def test_wrong_cli_version_is_rejected(self):
        self.archive = self._make_archive(reported_version="9.9.9")
        self._write_release(self.archive)
        result = self._run_build()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("version", result.stdout.lower())
        self.assertFalse((self.macos / "dist" / "GDirect.app").exists())

    def test_archive_without_ca_is_rejected(self):
        self.archive.unlink()
        self.archive = self._make_archive(include_ca=False)
        self._write_release(self.archive)
        result = self._run_build()
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("cacert.pem", result.stdout)
        self.assertFalse((self.macos / "dist" / "GDirect.app").exists())

    def test_empty_local_cli_is_rejected_instead_of_downloading(self):
        result = self._run_build("--cli", "")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("absolute", result.stdout)
        self.assertFalse((self.macos / "dist" / "GDirect.app").exists())

    def test_local_cli_is_validated_and_bundled_with_sibling_ca(self):
        cli = self._make_local_cli()
        result = self._run_build("--cli", str(cli))
        self.assertEqual(result.returncode, 0, result.stdout)
        bundled = self.macos / "dist" / "GDirect.app" / "Contents" / "MacOS"
        version = subprocess.run(
            [str(bundled / "gdirect"), "--version"],
            text=True,
            capture_output=True,
            check=True,
        )
        self.assertEqual(version.stdout.strip(), "gdirect 1.2.3")
        self.assertIn("local", (bundled / "cacert.pem").read_text())

    def test_local_cli_rejects_relative_path_and_wrong_version(self):
        cli = self._make_local_cli(version="0.0.1")
        relative = self._run_build("--cli", "local-cli/gdirect")
        self.assertNotEqual(relative.returncode, 0)
        self.assertIn("absolute", relative.stdout)
        wrong_version = self._run_build("--cli", str(cli))
        self.assertNotEqual(wrong_version.returncode, 0)
        self.assertIn("version", wrong_version.stdout.lower())

    def test_rebuild_replaces_only_app_and_preserves_unrelated_files(self):
        first = self._run_build()
        self.assertEqual(first.returncode, 0, first.stdout)
        dist = self.macos / "dist"
        unrelated = dist / "keep-me.txt"
        unrelated.write_text("do not delete")
        stale = dist / "GDirect.app" / "Contents" / "stale-file"
        stale.write_text("old app")
        second = self._run_build()
        self.assertEqual(second.returncode, 0, second.stdout)
        self.assertEqual(unrelated.read_text(), "do not delete")
        self.assertFalse(stale.exists())

    def test_failed_dist_verification_preserves_previous_app_and_cleans_staging(self):
        first = self._run_build()
        self.assertEqual(first.returncode, 0, first.stdout)
        dist = self.macos / "dist"
        old_marker = dist / "GDirect.app" / "Contents" / "previous-build"
        old_marker.write_text("preserve me")
        unrelated = dist / "keep-me.txt"
        unrelated.write_text("do not delete")

        failed = self._run_build(extra_env={"FAKE_FAIL_DIST_VERIFY": "1"})
        self.assertNotEqual(failed.returncode, 0)
        self.assertIn("injected dist verification failure", failed.stdout)
        self.assertTrue(old_marker.is_file(), "previous app must survive failed finalization")
        self.assertEqual(old_marker.read_text(), "preserve me")
        self.assertEqual(unrelated.read_text(), "do not delete")
        self.assertEqual(list(dist.glob(".GDirect.app.*")), [])

        recovered = self._run_build()
        self.assertEqual(recovered.returncode, 0, recovered.stdout)
        self.assertFalse(old_marker.exists())
        self.assertEqual(unrelated.read_text(), "do not delete")


if __name__ == "__main__":
    unittest.main()
