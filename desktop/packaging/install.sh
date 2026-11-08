#!/bin/sh
# Installs gdirect's desktop client for the current user: the program in
# ~/.local/bin, plus a menu entry and icon. Run it from the unpacked folder.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
data=${XDG_DATA_HOME:-$HOME/.local/share}
install -Dm755 "$here/gdirect-desktop" "$HOME/.local/bin/gdirect-desktop"
install -Dm644 "$here/gdirect.png" "$data/icons/hicolor/512x512/apps/gdirect.png"
mkdir -p "$data/applications"
sed "s|^Exec=.*|Exec=$HOME/.local/bin/gdirect-desktop|" "$here/gdirect.desktop" > "$data/applications/gdirect.desktop.tmp"
mv "$data/applications/gdirect.desktop.tmp" "$data/applications/gdirect.desktop"
echo "Installed. Open gdirect from your applications menu, or run ~/.local/bin/gdirect-desktop."
