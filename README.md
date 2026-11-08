# gdirect

[中文](README.zh-CN.md) · [Changelog](CHANGELOG.md)

gdirect browses, downloads, uploads and mounts your Google Drive from the terminal. It finds Google IPs that are reachable from your network and sends Drive traffic straight to them, with multi-connection, resumable downloads.

## How it works

| Traffic | Route |
| --- | --- |
| Drive API, downloads, uploads and their token refresh (`browse`, `ls`, `download`, `upload`) | A scanned Google IP. The hostname and certificate check stay the same. If connections carrying the real SNI are reset, gdirect automatically retries under a forged identity (`g.cn` as TLS name, the real host in the Host header) — same idea as Sheas-Cealer — and remembers what worked per IP |
| Sign-in and account verification (`accounts.google.com`, `oauth2.googleapis.com`, `www.googleapis.com`), and token refresh when no scanned IP answers for it | Your normal system network (DNS, proxy) |
| `mount` (rclone) | Your normal system network |

The first transfer command sweeps Google's own front ends and keeps the ones that work for 24 hours. They come from the address space Google publishes as its own ([goog.json minus cloud.json](https://knowledge.workspace.google.com/admin/security/obtain-google-ip-address-ranges), embedded and refreshed weekly). Most Google IPs are blocked in mainland China, but some front ends are not: the sweep tries a TCP connection first, fully verifies only what answers (TLS, the Drive API and a test download), and stops once it has 8. Each candidate is probed with the real SNI first, then with the forged identity. If an endpoint stops answering, requests and downloads move to the next verified one. `login` and `mount` do not scan. Certificates are verified against Google's own GTS roots (embedded in the binary), never against a system CA bundle — a compromised third-party CA cannot impersonate Google for gdirect.

Network requirements:

- Sign-in needs browser access to `accounts.google.com` and application access to `oauth2.googleapis.com` and `www.googleapis.com`. If Google is blocked, keep a proxy available for sign-in. During transfers, token refresh goes through the scanned IP too, so no proxy is needed then; only if that IP does not answer as Google's token endpoint does gdirect fall back to the system network.
- If you use a TUN-mode proxy or VPN (for example Clash/Mihomo TUN), route Google's IP ranges **DIRECT** and turn off fake-ip for them. A TUN proxy accepts every connection, so otherwise the scan measures the proxy instead of your network; `scan` warns when it sees this. `gdirect scan --list` shows the IPs in use.

### System proxy without TUN

- On macOS and Windows, sign-in and account verification use the manual HTTP/HTTPS system proxy (token refresh too, when it falls back to the system network). A proxy client in rule mode with TUN off is enough: proxy `accounts.google.com`, `oauth2.googleapis.com` and `www.googleapis.com`. No DIRECT rules are needed; scans, Drive API requests, uploads and downloads still connect to the scanned endpoints directly.
- If you turn TUN on with domain sniffing, a `www.googleapis.com` proxy rule also catches those transfers. Exclude the scanned IPs from sniffing, or send all of gdirect's traffic DIRECT with a process rule such as Clash's `PROCESS-NAME,gdirect,DIRECT`.
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

You can also download a [release archive](https://github.com/mizorewww/gdirect/releases/latest). Certificates are verified against Google's own GTS roots, which are embedded in the binary — no CA bundle file is needed.

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
gdirect upload ./photos -r                       # folder tree, resumes if interrupted
gdirect upload ./photos -r --parent FOLDER_ID --name "Photos backup"
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
| `u` | Upload a local file or folder into the current folder. Folders are uploaded recursively |
| `d` | Queue the marked items, or the selected one. A folder is downloaded with all its subfolders into `<output>/<folder name>/`. Google Docs/Sheets/Slides are exported as PDF |
| `+/-` | Connections for the next download (1–16), saved as your default |
| `c` | Cancel the current download and keep the partial data |
| `R` | Retry failed or cancelled downloads |
| `X` | Remove downloads that have not started yet |
| `q` | Quit |

- **Queue:** downloads run one at a time. The file list shows each file's state (◦ queued, ↓ downloading, ✓ saved, ✗ failed), and the Downloads panel shows overall progress. Select a failed file to see the full error. Files with the same name in one session are saved as `name (2).ext`.
- **Resume:** if a download or upload is interrupted, run the same command again. A folder upload reuses the folders it already created and skips files that finished.
- **Downloads** that already finished are recognised by their checksum and reported as already downloaded. Files are checked against Drive's MD5 after the download.
- **Uploads** always create a new file. They never overwrite a file with the same name.
- **Folder uploads:** `gdirect upload DIR -r` mirrors the local tree under a new Drive folder, including empty folders; symbolic links and special files are skipped. If the run is interrupted, the same command resumes. `--restart` starts over with a new Drive folder.
- **Connections:** `-j N` sets the number of connections for one command only. The saved default (8 at first) changes only with `+/-` in `browse`.

In `browse`, `u` accepts a local file or folder path. A folder is uploaded recursively and its progress is shown in the status bar.

Advanced route options (`download` and `browse` take all of them; `ls` and `upload` accept `--ip` and `--sni`):

| Option | Meaning |
| --- | --- |
| `--ip IP` | Use this Google IP instead of the scanned one |
| `--sni HOST` / `--no-sni` | Force a forged TLS identity (Host header stays real), or send no SNI. Without them gdirect uses the scanned identity and falls back automatically. `--no-sni` applies to downloads only |
| `--ca-cert FILE` | Use this CA bundle instead of the embedded Google GTS roots |
| `--export MIME` | (`download`) Export a Google Docs file, for example `application/pdf` |

Run `gdirect scan` to scan again and `gdirect scan --list` to see the results, including the last error for each IP. To test your own candidates, run `gdirect scan --ips FILE`. The file holds one public IP per line, and `#` starts a comment. Only addresses inside Google's own space are used; others are skipped, and so is `--ip` outside it. Third-party relays are not supported: they stop working without notice, and their operators see your IP address.

For scripts, `GDIRECT_ACCESS_TOKEN` can supply an access token instead of a login (`mount` still needs `gdirect login`).

## Multiple accounts

Sign in to more Google accounts with the same OAuth client:

```sh
gdirect login --add                # choose another account in the browser
gdirect accounts                   # * marks the default account
gdirect switch work                # make work@… the default
gdirect -a me@gmail.com browse     # use one account for a single command
gdirect logout work                # sign out one account (--all for every one)
```

Every command takes `-a`/`--account` with a full address or an unambiguous prefix; `GDIRECT_ACCOUNT` sets it for a shell or script. The account you last signed in with becomes the default, and `browse` shows the current account in its header. Each account has its own folder cache and upload sessions, and can be mounted at the same time as the others (pick a different directory). While your OAuth app is in Testing, every account must be added as a test user ([step 5](docs/configuration/google-cloud.md#5-allow-your-account-to-sign-in)).

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
- `mount -a ACCOUNT` mounts another account. `unmount` detaches the default account's mount, or the only one; with several mounts add `-a ACCOUNT`.

## Your data and logout

Credentials, scanned IPs and settings are stored in one private SQLite file:

- macOS: `~/Library/Application Support/gdirect/state.sqlite3`
- Linux: `~/.local/share/gdirect/state.sqlite3`
- Windows: `%LOCALAPPDATA%\gdirect\state.sqlite3`

Use `--db PATH` to choose another file. `gdirect logout` removes the default account's stored credentials (or the account you name; `--all` removes every account) and keeps the scanned IPs. Unmount that account before logging it out. To add or switch Google accounts, see [Multiple accounts](#multiple-accounts).

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
