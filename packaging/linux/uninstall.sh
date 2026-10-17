#!/bin/sh
set -eu
install_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
data_root=${XDG_DATA_HOME:-"$HOME/.local/share"}
expected_dir="$data_root/ScreenCue"
if [ "$install_dir" != "$expected_dir" ] || [ ! -f "$install_dir/package.json" ]; then
    printf '%s\n' 'Run the installed uninstall.sh from your user data ScreenCue directory.' >&2
    exit 1
fi
rm -f -- "$data_root/applications/screencue.desktop" "$data_root/icons/hicolor/256x256/apps/screencue.png"
# Only this explicitly verified application directory is removed. Preferences
# and conversation configuration outside it are preserved.
rm -rf -- "$install_dir"
printf '%s\n' 'ScreenCue removed. Your user preferences were preserved.'
