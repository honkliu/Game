#!/usr/bin/env bash
set -euo pipefail
if [[ "$(uname -s)" != Darwin ]]; then
    echo "Build the macOS application on a Mac with Xcode command line tools." >&2
    exit 1
fi
cd "$(dirname "$0")/.."
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-14.0}"
target="${1:-$(rustc -vV | sed -n 's/^host: //p')}"
case "$target" in
    aarch64-apple-darwin|x86_64-apple-darwin) ;;
    *) echo "Unsupported macOS target: $target" >&2; exit 1 ;;
esac
if [[ -n "${APPLE_NOTARY_PROFILE:-}" && -z "${APPLE_SIGNING_IDENTITY:-}" ]]; then
    echo "Notarization requires APPLE_SIGNING_IDENTITY (a Developer ID Application identity)." >&2
    exit 1
fi
cargo build --locked --release --target "$target" -p browser
dist="$PWD/dist/$target"
app="$dist/AI Browser.app"
mkdir -p "$app/Contents/MacOS"
cp tools/macos/Info.plist "$app/Contents/Info.plist"
cp "${CARGO_TARGET_DIR:-target}/$target/release/browser" "$app/Contents/MacOS/browser"
chmod +x "$app/Contents/MacOS/browser"
if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
    codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$app"
else
    codesign --force --sign - "$app"
fi
codesign --verify --strict "$app"
archive="$dist/AI Browser.zip"
ditto -c -k --keepParent "$app" "$archive"
if [[ -n "${APPLE_NOTARY_PROFILE:-}" ]]; then
    xcrun notarytool submit "$archive" --keychain-profile "$APPLE_NOTARY_PROFILE" --wait
    xcrun stapler staple "$app"
    ditto -c -k --keepParent "$app" "$archive"
fi
printf 'Application: %s\nArchive: %s\n' "$app" "$archive"
