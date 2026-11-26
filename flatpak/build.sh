#!/usr/bin/env bash
# Build SQLCraft as a Flatpak and install it for the current user.
#
#   flatpak/build.sh           build and install
#   flatpak/build.sh --bundle  also write SQLCraft.flatpak to copy to other machines
#
# Needs flatpak and flatpak-builder (Ubuntu/Debian: sudo apt install flatpak flatpak-builder).
set -euo pipefail
cd "$(dirname "$0")"

APP=io.github.loganhoney.SQLCraft
BRANCH=25.08

for tool in flatpak flatpak-builder python3; do
    command -v "$tool" >/dev/null || { echo "Missing $tool. On Ubuntu: sudo apt install flatpak flatpak-builder" >&2; exit 1; }
done

flatpak remote-add --user --if-not-exists flathub https://dl.flathub.org/repo/flathub.flatpakrepo
flatpak install --user -y --noninteractive flathub \
    "org.freedesktop.Platform//$BRANCH" "org.freedesktop.Sdk//$BRANCH" \
    "org.freedesktop.Sdk.Extension.rust-stable//$BRANCH" "org.freedesktop.Sdk.Extension.llvm21//$BRANCH"

# Keep the offline crate list in step with Cargo.lock.
python3 gen-cargo-sources.py

extra=()
[ "${1:-}" = "--bundle" ] && extra=(--repo=repo)
flatpak-builder --user --install --force-clean "${extra[@]}" build-dir "$APP.yml"

if [ "${1:-}" = "--bundle" ]; then
    flatpak build-bundle --runtime-repo=https://dl.flathub.org/repo/flathub.flatpakrepo repo ../SQLCraft.flatpak "$APP"
    echo "Wrote $(cd .. && pwd)/SQLCraft.flatpak (install with: flatpak install --user SQLCraft.flatpak)"
fi
echo "Run it with: flatpak run $APP"
