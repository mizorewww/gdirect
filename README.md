# gdirect

[中文](README.zh-CN.md) · [Changelog](CHANGELOG.md)

gdirect browses, downloads, uploads and mounts your Google Drive from the terminal. It finds Google IPs that are reachable from your network and sends Drive traffic straight to them, with multi-connection, resumable downloads.

## How it works

| Traffic | Route |
| --- | --- |
| Drive API, downloads, uploads (`browse`, `ls`, `download`, `upload`) | A scanned Google IP. The hostname and certificate check stay the same; the TLS SNI can be changed with `--sni` / `--no-sni` |
| Sign-in, account verification and token refresh (`accounts.google.com`, `oauth2.googleapis.com`, `www.googleapis.com`) | Your normal system network (DNS, proxy) |
| `mount` (rclone) | Your normal system network |

The first transfer command scans candidate IPs automatically and keeps the fastest working one for 24 hours. `login` and `mount` do not scan.

Network requirements:

- Sign-in needs browser access to `accounts.google.com` and application access to `oauth2.googleapis.com` and `www.googleapis.com`. Token refresh needs `oauth2.googleapis.com`. If Google is blocked, keep a proxy available for these requests, including during transfers.
- If you use a TUN-mode proxy or VPN (for example Clash/Mihomo TUN), route the scanned Google IPs **DIRECT** and turn off fake-ip for them. Otherwise gdirect's traffic is captured again and the scan finds nothing. `gdirect scan --list` shows the IPs.

### System proxy without TUN

- On macOS and Windows, sign-in, account verification and token refresh use the manual HTTP/HTTPS system proxy. A proxy client in rule mode with TUN off is enough: proxy `accounts.google.com`, `oauth2.googleapis.com` and `www.googleapis.com`. No DIRECT rules are needed; scans, Drive API requests, uploads and downloads still connect to the scanned IP directly.
- If you turn TUN on with domain sniffing, a `www.googleapis.com` proxy rule also catches those transfers. Exclude the scanned IPs from sniffing.
- On Linux, or with PAC, set `HTTPS_PROXY` for each command (your client's HTTP or mixed port). `HTTP_PROXY` / `HTTPS_PROXY` take precedence over system settings, and `mount` passes them on to rclone. `NO_PROXY` must not exclude the Google endpoints above.

```sh
HTTPS_PROXY=http://127.0.0.1:7890 gdirect login --client-secret ~/Downloads/client_secret.json
HTTPS_PROXY=http://127.0.0.1:7890 gdirect mount
```

## Install

Prebuilt for macOS 15+ (Apple Silicon / Intel), Linux x86_64 with glibc 2.35+ (Ubuntu 22.04+, Debian 12+, recent Fedora/Arch/openSUSE), and Windows x86_64. Alpine/musl, older glibc and Linux ARM64 need a [source build](#build-from-source).

macOS / Linux (requires `curl`):

```sh
curl -fsSL https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.sh | bash
```

Windows PowerShell:

```powershell
& ([scriptblock]::Create((Invoke-RestMethod 'https://raw.githubusercontent.com/mizorewww/gdirect/main/scripts/install.ps1')))
```

The installer puts gdirect in `~/.local/bin` (Windows: `%LOCALAPPDATA%\gdirect\bin`) and tells you how to add it to PATH. On Windows, open a new terminal after installing.

To use `mount`, add `--with-mount` (PowerShell: `-WithMount`). This also installs rclone 1.75.2+ and, on Linux, FUSE3 through your package manager (uses sudo). On Windows it installs rclone and WinFsp with winget; WinFsp may ask for administrator approval.

Homebrew (macOS / Linux):

```sh
brew tap mizorewww/gdirect https://github.com/mizorewww/gdirect
brew install mizorewww/gdirect/gdirect
brew install rclone   # only for mount; Linux also needs your distro's fuse3 package
```

Upgrade with `brew update && brew upgrade gdirect`, or by rerunning the installer.

You can also download a [release archive](https://github.com/mizorewww/gdirect/releases/latest). Keep `cacert.pem` next to the executable, because downloads use it to verify certificates.

## One-time Google setup (about 10 minutes)

gdirect does not ship a shared Google client, so you create your own in a Google Cloud project, once. Follow the [Google setup guide](docs/configuration/README.md) to download a **Desktop app** OAuth client JSON, then:

```sh
gdirect login --client-secret ~/Downloads/client_secret.json
```

A browser opens. After you approve, the terminal prints `Logged in`. The JSON file is not needed again.

## Daily use

```sh
gdirect                                   # same as gdirect browse
gdirect browse -o ~/Downloads             # download directory (default: ./downloads)
gdirect ls root --json
gdirect download FILE_ID_OR_URL           # saves to ./downloads/<name>
gdirect download FILE_ID -o file.zip
gdirect download DOC_ID --export application/pdf -o doc.pdf
gdirect upload ./file.zip
gdirect upload ./file.zip --parent FOLDER_ID --name backup.zip
```

`browse` shows the parent folder, the current folder and a preview side by side, like [yazi](https://github.com/sxyazi/yazi). Press `?` for every key.

| Key in `browse` | Action |
| --- | --- |
| `j/k`, `↑/↓` | Select |
| `g/G`, Home/End | First / last item |
| `Ctrl-d/Ctrl-u`, PgDn/PgUp | Half a page / a page down or up |
| Enter, `l/→` | Open folder |
| `h/←`, Backspace | Parent folder |
| `/` | Filter the current folder by name. Enter keeps the filter, Esc clears it |
| `Ctrl-r`, F5 | Refresh |
| Space | Mark or unmark an item. Marks are kept when you change folders |
| `Ctrl-a` | Mark everything shown (press again to unmark) |
| Esc | Clear the filter, then the marks, then the clipboard |
| `a` | Create an empty file; a name ending in `/` creates a folder |
| `r` | Rename |
| `y` / `x` | Copy / cut the marked items, or the selected one |
| `p` | Paste into the current folder. Cut items are moved; copied files are duplicated (Drive cannot copy folders) |
| `D`, Delete | Move the marked items, or the selected one, to the Drive trash (asks first) |
| `u` | Upload a local file into the current folder |
| `d` | Queue the marked items, or the selected one. A folder is downloaded with all its subfolders into `<output>/<folder name>/`. Google Docs/Sheets/Slides are exported as PDF |
| `+/-` | Connections for the next download (1–16), saved as your default |
| `c` | Cancel the current download and keep the partial data |
| `R` | Retry failed or cancelled downloads |
| `X` | Remove downloads that have not started yet |
| `q` | Quit |

- **Queue:** downloads run one at a time. The file list shows each file's state (◦ queued, ↓ downloading, ✓ saved, ✗ failed), and the Downloads panel shows overall progress. Select a failed file to see the full error. Files with the same name in one session are saved as `name (2).ext`.
- **Resume:** if a download or upload is interrupted, run the same command again.
- **Downloads** that already finished are recognised by their checksum and reported as already downloaded. Files are checked against Drive's MD5 after the download.
- **Uploads** always create a new file. They never overwrite a file with the same name.
- **Connections:** `-j N` sets the number of connections for one command only. The saved default (8 at first) changes only with `+/-` in `browse`.

Advanced transfer options for `download` and `browse` (`ls` and `upload` accept `--ip` only):

| Option | Meaning |
| --- | --- |
| `--ip IP` | Use this Google IP instead of the scanned one |
| `--sni HOST` / `--no-sni` | Send a different TLS SNI, or none. Certificates are still checked against the real hostname |
| `--ca-cert FILE` | Use this CA bundle instead of the bundled `cacert.pem` |
| `--export MIME` | (`download`) Export a Google Docs file, for example `application/pdf` |

Run `gdirect scan` to scan again and `gdirect scan --list` to see the results, including the last error for each IP. To test your own candidates, run `gdirect scan --ips FILE`. The file holds one public IP per line, and `#` starts a comment.

For scripts, `GDIRECT_ACCESS_TOKEN` can supply an access token instead of a login (`mount` still needs `gdirect login`).

## Mount

Mounting requires the dependencies from `--with-mount` or the Homebrew step above.

```sh
gdirect mount                        # ~/GoogleDrive
gdirect mount ~/MyDrive --read-only
gdirect unmount
```

- On macOS the mount uses the built-in NFS client, so no FUSE driver is needed. Linux uses FUSE3.
- On Windows the mount runs in the foreground and stops with Ctrl-C.
- The mount directory must be empty on macOS/Linux. On Windows it must not exist yet.
- Writes go to a local cache first and upload in the background. The cache tries to stay under 5 GiB, but open or unsent files can exceed that. Uploads are finished when the log printed by `mount` shows `to upload 0, uploading 0`.
- `unmount` keeps files that have not been uploaded yet, and the next `mount` uploads them.
- `-j` sets how many rclone transfers run at once.

## Your data and logout

Credentials, scanned IPs and settings are stored in one private SQLite file:

- macOS: `~/Library/Application Support/gdirect/state.sqlite3`
- Linux: `~/.local/share/gdirect/state.sqlite3`
- Windows: `%LOCALAPPDATA%\gdirect\state.sqlite3`

Use `--db PATH` to choose another file. `gdirect logout` removes the stored credentials and keeps the scanned IPs. Unmount before logging out. To switch Google accounts, run `gdirect login --reauth`.

Sign-in problems, the 7-day expiry and other questions are covered in [Troubleshooting](docs/configuration/README.md#troubleshooting).

## Build from source

Requires Rust 1.91+, a C++ compiler, make, curl and OpenSSL 3.

```sh
git clone https://github.com/mizorewww/gdirect.git
cd gdirect
# macOS: brew install openssl@3
# Debian / Ubuntu: sudo apt-get install build-essential pkg-config libssl-dev ca-certificates curl xz-utils
./scripts/build-libaria2.sh
cargo install --path . --locked
```

Windows builds use the MinGW GNU toolchain. In an MSYS2 MINGW64 shell, install `make patch tar xz mingw-w64-x86_64-gcc mingw-w64-x86_64-openssl mingw-w64-x86_64-pkgconf`, run `rustup target add x86_64-pc-windows-gnu`, set `OPENSSL_PREFIX=$(cygpath -m /mingw64)`, then run the two commands above with `--target x86_64-pc-windows-gnu` added to `cargo install`.
