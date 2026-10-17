#!/bin/sh
set -eu
source_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
data_root=${XDG_DATA_HOME:-"$HOME/.local/share"}
install_dir="$data_root/ScreenCue"
case "$install_dir" in /*/ScreenCue) ;; *) printf '%s\n' 'Install location must be absolute.' >&2; exit 1;; esac
mkdir -p "$install_dir" "$data_root/applications" "$data_root/icons/hicolor/256x256/apps"
for name in screencue screencue-listen LICENSE README.md package.json app-icon.png uninstall.sh; do
    cp "$source_dir/$name" "$install_dir/$name"
done
for name in licenses docs conversation controller; do
    if [ -d "$source_dir/$name" ]; then cp -R "$source_dir/$name" "$install_dir/"; fi
done
chmod 755 "$install_dir/screencue" "$install_dir/screencue-listen" "$install_dir/uninstall.sh"
cp "$source_dir/app-icon.png" "$data_root/icons/hicolor/256x256/apps/screencue.png"
# Desktop Exec quoting also escapes backticks, dollar signs and percent field codes.
escaped_dir=$(printf '%s' "$install_dir" | sed 's/\\/\\\\/g; s/"/\\"/g; s/`/\\`/g; s/\$/\\$/g; s/%/%%/g')
cat > "$data_root/applications/screencue.desktop" <<EOF
[Desktop Entry]
Type=Application
Version=1.0
Name=ScreenCue
Comment=Desktop hints and conversation notes
Exec="$escaped_dir/screencue" --app
Icon=screencue
Terminal=false
Categories=Utility;Accessibility;
StartupNotify=false
EOF
if command -v update-desktop-database >/dev/null 2>&1; then update-desktop-database "$data_root/applications" || true; fi
printf 'Installed ScreenCue to %s\nOpen ScreenCue from your application menu.\n' "$install_dir"
