#!/bin/sh

# You might need to restart your pc if sharun doesn't create `AppDir` in this directory (It should create dirs on its own)

# Run git.yml and extract the pkg.tar.zst generated from that to the folder this .sh file is in. As a result a folder named `usr` should be in the same folder as this script.
set -eu

ARCH="$(uname -m)"
SHARUN="https://raw.githubusercontent.com/pkgforge-dev/Anylinux-AppImages/bc2acd96e07614d36ad9b206b053f7cf45e41720/useful-tools/quick-sharun.sh"

#export UPINFO="gh-releases-zsync|${GITHUB_REPOSITORY%/*}|${GITHUB_REPOSITORY#*/}|latest|*$ARCH.AppImage.zsync"
export OUTNAME=OverBind-anylinux-"$ARCH".AppImage
export DESKTOP=./usr/share/applications/OverBind.desktop
export ICON=./usr/share/icons/hicolor/256x256@2/apps/OverBind.png
export DEPLOY_OPENGL=0
export DEPLOY_VULKAN=0
export DEPLOY_DOTNET=0

#Remove leftovers
rm -rf AppDir dist appinfo

# ADD LIBRARIES
wget --retry-connrefused --tries=30 "$SHARUN" -O ./quick-sharun
chmod +x ./quick-sharun

# Point to binaries and resource directories
./quick-sharun ./usr/bin/OverBind ./usr/bin/cursor-overlay-$(uname -m)-unknown-linux-gnu ./usr/lib/OverBind

# Make AppImage
./quick-sharun --make-appimage

mkdir -p ./dist
mv -v ./*.AppImage* ./dist

echo "All Done!"
