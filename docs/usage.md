# Using gdirect

[中文](usage.zh-CN.md) · [README](../README.md)

## Install

Prebuilt for macOS 15+ (Apple Silicon / Intel), Linux x86_64 with glibc 2.35+ (Ubuntu 22.04+, Debian 12+, recent Fedora/Arch/openSUSE), and Windows x86_64. Alpine/musl, older glibc and Linux ARM64 need a [source build](building.md).

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
brew trust mizorewww/gdirect   # Homebrew 7 asks you to trust a third-party tap
brew install mizorewww/gdirect/gdirect
brew install --cask mizorewww/gdirect/gdirect-desktop   # the desktop app, macOS
brew install rclone   # only for mount; Linux also needs your distro's fuse3 package
```

Upgrade with `brew update && brew upgrade gdirect`, or by rerunning the installer.

You can also download a [release archive](https://github.com/mizorewww/gdirect/releases/latest). Certificates are verified against Google's own GTS roots, which are embedded in the binary — no CA bundle file is needed.

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

Run `gdirect scan` to scan again and `gdirect scan --list` to see the results, including the last error for each IP. To test your own candidates, run `gdirect scan --ips FILE`. The file holds one public IPv4 or IPv6 address per line, and `#` starts a comment. Only addresses inside Google's own space are used; others are skipped, and so is `--ip` outside it. Third-party relays are not supported: they stop working without notice, and their operators see your IP address.

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

Every command takes `-a`/`--account` with a full address or an unambiguous prefix; `GDIRECT_ACCOUNT` sets it for a shell or script. The account you last signed in with becomes the default, and `browse` shows the current account in its header. Each account has its own folder cache and upload sessions, and can be mounted at the same time as the others (pick a different directory). While your OAuth app is in Testing, every account must be added as a test user ([step 5](configuration/google-cloud.md#5-allow-your-account-to-sign-in)).

## Mount

Mounting requires the dependencies from `--with-mount` or the Homebrew step above.

```sh
gdirect mount                        # ~/GoogleDrive
gdirect mount ~/MyDrive --read-only
gdirect unmount
```

- rclone reaches Google through the scanned IPs, so mounting needs no proxy: gdirect runs a local proxy that only forwards Google hosts to them and lives as long as rclone. IPs verified with the real hostname go first, since rclone's TLS carries it. `--ip` pins one.
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

Sign-in problems, the 7-day expiry and other questions are covered in [Troubleshooting](configuration/README.md#troubleshooting).
