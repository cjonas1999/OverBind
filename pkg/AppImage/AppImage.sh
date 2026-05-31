#!/bin/sh

# You might need to restart your pc if sharun doesn't create `AppDir` in this directory (It should create dirs on its own)

# Run git.yml and extract the pkg.tar.zst generated from that to the folder this .sh file is in. As a result a folder named `usr` should be in the same folder as this script.
set -eu

ARCH="$(uname -m)"
SHARUN_REPO="https://raw.githubusercontent.com/pkgforge-dev/Anylinux-AppImages"

# Resolve a "settled" commit: the newest commit to useful-tools/ that is
# >=24h old and was not followed by another commit within 24h of its timestamp.
# This avoids blindly running whatever is on main while still tracking updates.
_commits_json=$(mktemp)
wget -qO "$_commits_json" \
    "https://api.github.com/repos/pkgforge-dev/Anylinux-AppImages/commits?path=useful-tools/&per_page=100"

STABLE_SHA=$(node -e "
const data = JSON.parse(require('fs').readFileSync('$_commits_json', 'utf8'));
const now = Date.now(), h24 = 864e5;
for (let i = 0; i < data.length; i++) {
    const t = new Date(data[i].commit.committer.date).getTime();
    if (now - t < h24) continue;
    const prev = data[i - 1];
    if (!prev || new Date(prev.commit.committer.date).getTime() - t >= h24) {
        process.stdout.write(data[i].sha);
        process.exit(0);
    }
}
process.stderr.write('No settled commit found in last 100 commits\n');
process.exit(1);
")
rm -f "$_commits_json"

echo "Pinning sharun scripts to settled commit $STABLE_SHA"
SHARUN="$SHARUN_REPO/$STABLE_SHA/useful-tools/quick-sharun.sh"
DEBLOATED_PKGS="$SHARUN_REPO/$STABLE_SHA/useful-tools/get-debloated-pkgs.sh"

#export UPINFO="gh-releases-zsync|${GITHUB_REPOSITORY%/*}|${GITHUB_REPOSITORY#*/}|latest|*$ARCH.AppImage.zsync"
export OUTNAME=OverBind-anylinux-"$ARCH".AppImage
export DESKTOP=/usr/share/applications/OverBind.desktop
export ICON=/usr/share/icons/hicolor/256x256@2/apps/OverBind.png
export DEPLOY_OPENGL=1

#Remove leftovers
rm -rf AppDir dist appinfo

# ADD LIBRARIES
wget --retry-connrefused --tries=30 "$DEBLOATED_PKGS" -O ./get-debloated-pkgs
wget --retry-connrefused --tries=30 "$SHARUN" -O ./quick-sharun
chmod +x ./quick-sharun ./get-debloated-pkgs

# Debloated pkgs
./get-debloated-pkgs --add-common --prefer-nano

# Point to binaries and resource directories
./quick-sharun \
        /usr/bin/OverBind        \
        /usr/bin/cursor-overlay* \
        /usr/lib/libayatana-appindicator*.so* \
        /usr/lib/OverBind

# Make AppImage
./quick-sharun --make-appimage

mkdir -p ./dist
mv -v ./*.AppImage* ./dist

echo "All Done!"
