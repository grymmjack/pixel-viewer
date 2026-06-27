#!/usr/bin/env bash
# Install pixelview's desktop entry + icon so KDE/Wayland shows a real task icon
# (Wayland keys the task-switcher icon off app_id -> a matching .desktop file).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
DATA="${XDG_DATA_HOME:-$HOME/.local/share}"
ICON_DIR="$DATA/icons/hicolor/256x256/apps"
APP_DIR="$DATA/applications"

mkdir -p "$ICON_DIR" "$APP_DIR"
install -m644 "$HERE/assets/pixelview.png" "$ICON_DIR/pixelview.png"
install -m644 "$HERE/pixelview.desktop" "$APP_DIR/pixelview.desktop"

# Point Exec at the built binary if pixelview isn't already on PATH.
if ! command -v pixelview >/dev/null 2>&1; then
    BIN="$HERE/target/release/pixelview"
    if [ -x "$BIN" ]; then
        sed -i "s|^Exec=pixelview|Exec=$BIN|" "$APP_DIR/pixelview.desktop"
    fi
fi

command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$APP_DIR" || true
command -v gtk-update-icon-cache  >/dev/null 2>&1 && gtk-update-icon-cache -f "$DATA/icons/hicolor" 2>/dev/null || true

echo "Installed:"
echo "  $APP_DIR/pixelview.desktop"
echo "  $ICON_DIR/pixelview.png"
echo "Log out/in (or restart plasmashell/kwin) if the task icon doesn't refresh immediately."
