# Changelog

## Unreleased

### Fixed

- Sign-in, account verification and token refresh honor macOS and Windows HTTP/HTTPS system proxy settings without requiring TUN mode. Pinned Drive API requests, uploads, downloads and IP scans continue to bypass application proxies.
- Account verification during `login` now uses `HTTP_PROXY` / `HTTPS_PROXY` and system proxies instead of always connecting directly.

## 0.4.0

### Added

- File operations in `browse`, with yazi-style keys: `a` creates a file or (with a trailing `/`) a folder, `r` renames, `y`/`x` copy or cut and `p` pastes into the current folder, `D` moves items to the Drive trash after a confirmation, and `u` uploads a local file into the current folder.
- `/` filters the current folder by name; `g/G`, `Ctrl-d/Ctrl-u` and PgUp/PgDn move faster; `?` shows every key.
- A redesigned `browse`: parent, current folder and preview columns, files coloured by type, a breadcrumb header and a status bar with the mode, position, marks and clipboard.

### Changed

- `browse` keys that now belong to file operations moved: mark all is `Ctrl-a` (was `a`), refresh is `Ctrl-r`/F5 (was `r`), and removing queued downloads is `X` (was `x`).
- Mark all also marks folders, and any item can be marked.

## 0.3.0

### Added

- `browse` can queue several files at once: Space marks a file or folder, `a` marks every file in the current folder, Esc clears marks, and `d` queues everything marked.
- `d` on a folder downloads it with all subfolders into `<output>/<folder name>/`, keeping the folder structure.
- The file list shows each file's size and download state. The Downloads panel lists every job with overall progress, and the preview shows the full error of a failed download.
- `R` retries failed or cancelled downloads; `x` removes downloads that have not started.

### Fixed

- `browse` crashed with a segmentation fault when the second download started. libaria2 is now initialised once per process.
- Files with the same name queued in one session no longer collide; later ones are saved as `name (2).ext`.

## 0.2.0

### Breaking changes

- The top-level `gdirect FILE` download form is removed. Use `gdirect download FILE`. A mistyped subcommand now reports an unknown subcommand instead of starting a download.
- `--auth` / `--gcloud` are removed. Authentication is automatic after `gdirect login` or when `GDIRECT_ACCESS_TOKEN` is set.
- Transfer options are now accepted only by the commands that use them:
  - `download`: `--ip`, `--sni`, `--no-sni`, `--ca-cert`, `-j`, `-o FILE`, `--export MIME`
  - `browse`: `--ip`, `--sni`, `--no-sni`, `--ca-cert`, `-j`, `-o DIR`
  - `ls`, `upload`: `--ip`
  - `mount`: `-j`

  `--db` is still global.
- `--sni off` is replaced by `--no-sni`.
- `-j` on the command line applies to that command only and is no longer saved. The saved default changes only with `+/-` in `browse`.
- Google application-default credentials are no longer picked up automatically. Import them explicitly with `gdirect login --credentials PATH`.

### Added

- `gdirect logout` removes stored credentials and keeps scanned IPs.
- Downloads are checked against Drive's MD5 checksum. A file that is already complete is reported as already downloaded instead of failing.
- `gdirect scan --list` shows the last error for unavailable IPs.
- Help text for every option.

### Fixed

- `login`, `logout`, `mount` and `unmount` no longer run an IP scan first, so sign-in works even when no direct IP is reachable.
- Stored IPs that have not worked for 7 days are pruned. Previously the list grew until every scan failed with "at most 256 candidates".
- An IP that passes the Drive API check is no longer rejected just because the public test file is unavailable.
- Token refresh errors keep Google's reason, for example an expired or revoked refresh credential versus a network error.
- Drive API errors keep their HTTP status, and the re-login hint appears only for permission errors.
- Folder listings keep Drive's natural name order (`file2` before `file10`).
- Public downloads are only checked for Drive's HTML confirmation page when the saved file is actually HTML.
- `unmount`, and a failed mount, delete the credentials file exported for rclone. A foreground mount stopped with Ctrl-C on macOS/Linux still leaves it until the next mount.
- Shortcuts and Google Forms/Sites/Maps are no longer offered for PDF export in `browse`.
- Download failures show one short message for the actual error.
- Uploads use 32 MiB chunks, which is faster on high-latency links.

## 0.1.2

- HTTP downloads no longer depend on OpenSSL's legacy provider module.

## 0.1.1

- Built-in Google sign-in (`gdirect login --client-secret`). The Google Cloud CLI (gcloud) is no longer required.

## 0.1.0

- First release. Authentication relied on the Google Cloud CLI (gcloud).
