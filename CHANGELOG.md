# Changelog

## Unreleased

### Changed

- Desktop client: signing in starts from a form for the OAuth client ID and secret (the secret masked, with a show/hide toggle and inline checks), filled in by hand, from the downloaded JSON file, or from the clipboard; pasting the whole JSON, or the ID and secret together, into the Client ID field fills in both. Each field says where to find its value and links to the setup guide; a saved client is filled in already. Add account opens the same form in a dialog, and a sign-in waiting for the browser can be cancelled.
- Desktop client: Settings › Account shows the saved OAuth client (secret masked) and can copy it as JSON, export it to a private file, or change it.
- `gdirect login --client-id ID` signs in without the JSON file, reading the secret from `GDIRECT_CLIENT_SECRET` or asking for it with the input hidden; `gdirect login --export-client PATH|-` writes the saved client back out as a client JSON. Client JSON is read more tolerantly (a flat `client_id`/`client_secret` object, a byte-order mark), and a web client gets a clear error.
- Setup guide: a section on copying the client ID and secret, and on adding a new secret when the original was not kept.

## 0.13.0

### Added

- Desktop client: Shared with me, Starred, Recent, Trash and shared drives as places in the navigation rail, which expands beside the content from 1200 px wide (with a toggle it remembers) and opens over it in narrower windows. Storage use shows at the foot of the rail and in the account menu.
- Desktop client: search across all of Drive from the top of the Files page, with type, owner, modified-date and full-text filters and a Location column; "Filter this folder" stays for the current listing.
- Desktop client: Trash actions (restore, delete forever, empty trash), Undo after moving to trash or moving files (snackbar or ⌘Z), Copy link, Open in browser, Star, and Share; menus show only what Drive allows for each file. Google Docs, Sheets and Slides open in the browser and shortcuts act on their target. Big folders load page by page.
- Desktop client: a menu bar with File, Edit, View, Go, Window and Help menus and keyboard shortcuts throughout (⌘Q, ⌘1–4, ⌘F, ⌘[ / ⌘], ⇧⌘D/S/R/T for places, arrows, Home/End, Enter, ⌘⌫, F2…), with Finder-style selection. Settings › Keyboard shortcuts lists them all and lets you rebind, reset and resolve clashes.
- Desktop client: the window remembers its size and position.
- Arch Linux packages in the AUR: `gdirect-bin` and `gdirect-desktop-bin` from the release assets, `gdirect-git` and `gdirect-desktop-git` built from the repository. Releases update them automatically.

### Changed

- With a TUN-mode proxy or VPN on (sing-box, Clash/Mihomo, WireGuard…), gdirect's connections to Google front ends now leave through the physical network adapter instead of the tunnel, the way sing-box's `auto_detect_interface` escapes its own TUN (macOS `IP_BOUND_IF`, Linux `SO_BINDTODEVICE` with `CAP_NET_RAW`, Windows `IP_UNICAST_IF`). Previously the TUN sniffed the SNI and sent "direct" transfers through the proxy, and the scan measured the proxy, keeping endpoints that were unreachable directly. Scans, Drive API requests, uploads, aria2 downloads, token refresh over endpoints and the proxy `mount` runs for rclone are bound; sign-in and the system-network fallbacks still follow the proxy. On by default: `gdirect scan --bypass-tun off` (or Settings › Network in the desktop client) lets proxy rules decide, and `GDIRECT_BYPASS_TUN=0`/`1` overrides it for one command.
- Saved endpoints remember the network path they were verified over; after switching the setting, changing adapter, or a TUN coming or going while bypassing is off, they are scanned again before use. `gdirect scan --list` shows how connections leave the machine.
- Desktop client: the Clash process rule and the "route Google DIRECT" advice in Settings › Network give way to the bypass switch, which shows the adapter in use. The scan's interception warning now points to it.
- Desktop client: right-click menus everywhere, a responsive Settings page, rclone found outside `PATH` (Homebrew on macOS), clearer sign-in steps, a Chinese-first font for full-width punctuation, natural name sorting, and many layout, focus and wording fixes from several rounds of review.

## 0.12.1

### Added

- Desktop client downloads for every platform: a Windows installer (`gdirect-desktop-windows-x64.msi`: Program Files, a Start menu shortcut, in-place upgrades), a Linux tarball (`gdirect-desktop-linux-x64.tar.gz`, with `install.sh` for a menu entry) and the macOS apps. Their names no longer carry the version, so the README's download buttons always fetch the latest release.
- Homebrew cask for the desktop app: `brew install --cask mizorewww/gdirect/gdirect-desktop`.

## 0.12.0

### Added

- Desktop client for macOS on the release page: `gdirect-desktop-<version>-aarch64-apple-darwin.zip` and `…-x86_64-apple-darwin.zip` hold a `gdirect.app` (not notarized; allow it once in System Settings). Linux and Windows still build it from source.
- Desktop client: an English interface. It follows the system language (Chinese for Chinese locales, English otherwise) and can be pinned in Settings › Appearance.

### Changed

- Desktop client: the layout adapts to the window width — columns, the toolbar and the endpoint table give way as the window narrows, and settings switch to one pane at a time — and the window can shrink to 600 × 480.

### Fixed

- Downloads under the forged TLS identity kept the real Host header in one process-wide slot, so with several connections a redirect on one could give another the wrong Host header, and with it the OAuth token on a Drive download host. Each connection now keeps its own.

## 0.11.0

### Added

- A desktop client (preview) in `desktop/`, built with gpui-kit-material and Material Symbols: browse, filter, sort and manage files (new folder, rename, copy, move, trash), queue downloads and uploads (drag files in to upload) with live progress, cancel and resume, scan and pin endpoints (with the TUN interception warning), and manage accounts, mount, download options and appearance. It shares the command line's engine and state. It is built from source and needs gpui-kit-material checked out beside this repository; the release binaries remain the command line only. See "Build from source".

### Changed

- The engine is now a library crate (`gdirect`) that the command and the desktop client both use. `scan` reports its progress through events and can be cancelled, including while it verifies a batch; uploads can be cancelled between chunks and resume later. The command line's behaviour and output are unchanged.

## 0.10.0

### Removed

- Third-party relays. The built-in relay 183.56.143.147, tried first since 0.8.0, now refuses connections (thanks to readers who reported it). gdirect now uses only Google's own front ends: stored endpoints outside Google's address space are ignored, `scan --ips` skips them and `--ip` rejects them. `--google-only` and `GDIRECT_GOOGLE_ONLY` are still accepted but have no effect.

### Changed

- `scan` no longer starts from a built-in list of /24s that worked in testing; such lists go stale. It starts from the endpoints it verified before, then sweeps Google's core front-end blocks. A random batch of those already holds several that work from mainland China, so only the very first scan takes a little longer.

### Added

- `mount` no longer needs a proxy: rclone reaches Google through the scanned IPs via a local proxy that gdirect runs alongside it. The proxy only forwards Google hosts on port 443, TLS stays end to end with the real hostname, and IPs verified with that hostname are used first. `mount` takes `--ip` like the other commands, and falls back to the system network when no IP can be found. Before, mounting from mainland China without a proxy showed no files.
- IPv6: when the machine has a global IPv6 address, `scan` also sweeps Google's IPv6 front ends, following the address pattern of its regional front ends and moving on to the rest of each /64 that verifies. IPv6 candidates are mixed into the IPv4 sweep, so a network that blocks IPv6 still finds IPv4 endpoints as fast. `--ip` and `scan --ips` accept IPv6 addresses, and downloads work over them. Whether Google's IPv6 front ends are reachable from mainland China has not been measured yet.
- Recursive folder upload: `gdirect upload DIR --recursive` mirrors a local folder tree under a new Drive folder, including empty folders. An interrupted run resumes with the same command — folders already created are reused and finished files are skipped — and `--restart` starts over with a new Drive folder. Symbolic links and special files inside the folder are skipped. In `browse`, `u` now accepts a folder path and shows live upload progress in the status bar. (Thanks to @December172, #9.)

## 0.9.0

### Changed

- Token refresh during `browse`, `ls`, `download` and `upload` goes to `oauth2.googleapis.com` through the same scanned Google IP as the transfer, so it no longer needs a proxy. If that IP does not answer as Google's token endpoint, gdirect falls back to the system network as before. Sign-in and `mount` are unchanged. (Thanks to @tactfullotto, #4.)

### Fixed

- Uploads failed with 404 since 0.5.0: the upload URL repeated `/drive/v3/`. (Thanks to @tkxxx33, #6.)

## 0.8.0

### Changed

- The known relay 183.56.143.147 is used again, and first: it is not Google, but it is fast and stable in mainland China, and TLS through it is still verified end to end against Google's roots. When it fails, requests and downloads fall back to the verified Google front ends, which `scan` still collects. The 0.7.0 purge of it from databases is gone.

### Added

- `--google-only` (or `GDIRECT_GOOGLE_ONLY=1`) skips third-party relays: `scan` does not probe them and transfers use only endpoints inside Google's address space.

### Fixed

- Sign-in, account verification and token refresh honor macOS and Windows HTTP/HTTPS system proxy settings without requiring TUN mode. Pinned Drive API requests, uploads, downloads and IP scans continue to bypass application proxies. (Thanks to @Liuycer, #1.)
- Account verification during `login` now uses `HTTP_PROXY` / `HTTPS_PROXY` and system proxies instead of always connecting directly.

## 0.7.0

### Changed

- `scan` now sweeps Google's own front ends instead of starting from a hard-coded seed. That seed, 183.56.143.147, turned out to be a third-party reverse proxy (Google saw its Hong Kong exit), and it is purged from existing databases. Candidates come only from Google's published address space (goog.json minus cloud.json), embedded and refreshed weekly; poisoned DNS answers outside it are ignored. The sweep checks TCP first, fully verifies what answers, tries the rest of a working /24 next and stops at 8 verified endpoints. In a test from China Unicom (Shanghai) it found genuine Google front ends serving Drive directly at 4–6 MB/s per connection.
- Requests and downloads fail over to the next verified endpoint when the current one stops answering; downloads resume the partial file there.

### Added

- `scan` and `scan --list` mark endpoints outside Google's address space as `[third-party relay]`, and `scan` warns when a TUN proxy appears to be accepting every connection.

### Fixed

- `download --no-sni` and `browse --no-sni` no longer fail at start.

## 0.6.0

### Added

- Multiple Google accounts in one database. `gdirect login --add` (alias of `--reauth`) signs in another account with the saved OAuth client; `gdirect accounts` lists them, `gdirect switch ACCOUNT` changes the default, and the global `-a`/`--account` option (or `GDIRECT_ACCOUNT`) picks one for a single command by address or unambiguous prefix. `browse` shows the current account in its header.
- Each account has its own folder cache, upload sessions and background mount, so several accounts can be mounted at once.

### Changed

- `gdirect logout` signs out only the default account (or `logout ACCOUNT`); `--all` signs out every account. The saved OAuth client is removed once no account is left.
- Existing databases are migrated automatically: the current login becomes the default account. Unfinished uploads from older versions start over.

## 0.5.0

### Added

- Automatic forged-identity fallback on every pinned-IP path: when connections carrying the real Google SNI fail, requests retry under `g.cn` as TLS name with the real host kept in the HTTP Host header. The identity that works is stored per endpoint; `--sni`/`--no-sni` now apply to `ls` and `upload` too, and `--no-sni` remains download-only.
- `scan` probes the forged identity when the real one fails and `scan --list` shows which identity each IP needed.

### Changed

- Google-bound TLS now verifies against Google's own GTS roots embedded in the binary instead of a full CA bundle, so a compromised or mis-issuing third-party CA can no longer impersonate Google for gdirect. The `cacert.pem` side file is gone; `--ca-cert` remains an explicit override.
- A download that fails on the wire marks its endpoint dead, so the next command rescans instead of reusing a blocked IP for up to 24 hours.

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
