#!/usr/bin/env bash
# Build an ad-hoc preview candidate locally; this does not publish a release.
set -euo pipefail
cd "$(dirname "$0")/.."
[[ "$(uname -s)" == Darwin ]] || { echo 'Run this script on macOS.' >&2; exit 1; }
git diff --quiet -- crates apps Cargo.toml Cargo.lock
git diff --cached --quiet -- crates apps Cargo.toml Cargo.lock
revision=$(git rev-parse HEAD)
architecture=$(uname -m)

npm --prefix apps/desktop ci
python3 scripts/license_inventory.py
# Regenerated notices must belong to the recorded source commit.
git diff --quiet -- apps/desktop/src-tauri/resources docs/DEPENDENCY_LICENSES.md
cargo audit
cargo deny check licenses
(cd apps/desktop && npm run tauri build -- --bundles app --ci --no-sign -- --locked)

app="$PWD/target/release/bundle/macos/Klyndb.app"
cp LICENSE THIRD_PARTY_NOTICES.md "$app/Contents/Resources/"
codesign --force --deep --sign - "$app"
codesign --verify --deep --strict "$app"
[[ "$(lipo -archs "$app/Contents/MacOS/klyndb")" == "$architecture" ]]
if otool -L "$app/Contents/MacOS/klyndb" | sed '1d' | rg -q '/(opt|Users|usr/local)/'; then
  echo 'The executable depends on a development-machine library.' >&2; exit 1
fi
version=$(/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' "$app/Contents/Info.plist")
output="$PWD/artifacts/release-candidates/$version-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$output"
staging=$(mktemp -d "${TMPDIR:-/tmp}/klyndb-package.XXXXXX")
mounted=$(mktemp -d "${TMPDIR:-/tmp}/klyndb-mount.XXXXXX")
cleanup() {
  hdiutil detach "$mounted" >/dev/null 2>&1 || true
  rm -rf "$staging" "$mounted"
}
trap cleanup EXIT

ditto "$app" "$staging/Klyndb.app"
ln -s /Applications "$staging/Applications"
cp LICENSE THIRD_PARTY_NOTICES.md "$staging/"
cat > "$staging/READ_ME.txt" <<'NOTES'
Klyndb development preview — macOS native build

Copy Klyndb.app to Applications. Keep the existing app closed while replacing it.
Saved connections and workspace data are outside the application bundle.

This candidate is locally ad-hoc signed, without Developer ID signing or Apple
notarization. This signature does not establish Gatekeeper distribution approval.
Native release UI acceptance is required before publishing a validated release.
Windows, Linux, Intel Macs and other macOS versions are not verified by this build.

Source, build instructions and current limits:
https://github.com/OthmaneBlial/klyndb

No account, cloud connection or query telemetry is required.
The application includes locked third-party license texts in Contents/Resources.
NOTES
base="klyndb-$version-macos-$architecture"
ditto -c -k --sequesterRsrc --keepParent "$app" "$output/$base.app.zip"
hdiutil create -quiet -fs HFS+ -format UDZO -volname "Klyndb $version" -srcfolder "$staging" "$output/$base.dmg"
hdiutil verify "$output/$base.dmg"
hdiutil attach -quiet -readonly -nobrowse -mountpoint "$mounted" "$output/$base.dmg"
codesign --verify --deep --strict "$mounted/Klyndb.app"
cmp "$app/Contents/MacOS/klyndb" "$mounted/Klyndb.app/Contents/MacOS/klyndb"
cmp apps/desktop/src-tauri/resources/THIRD_PARTY_LICENSES.txt "$mounted/Klyndb.app/Contents/Resources/resources/THIRD_PARTY_LICENSES.txt"
cmp LICENSE "$mounted/Klyndb.app/Contents/Resources/LICENSE"
cmp THIRD_PARTY_NOTICES.md "$mounted/Klyndb.app/Contents/Resources/THIRD_PARTY_NOTICES.md"
[[ "$(readlink "$mounted/Applications")" == /Applications ]]
python3 - "$app" "$output" "$revision" "$architecture" <<'PY'
import hashlib, json, plistlib, platform, sys, zipfile
from pathlib import Path
app, output = map(Path, sys.argv[1:3])
info = plistlib.loads((app / 'Contents/Info.plist').read_bytes())
assert info['CFBundleIdentifier'] == 'io.klyndb.desktop'
assert info['CFBundleExecutable'] == 'klyndb'
artifacts = {}
for path in sorted(output.iterdir()):
    if path.name.endswith('.zip'):
        with zipfile.ZipFile(path) as archive:
            assert archive.testzip() is None
            for notice in ['LICENSE', 'THIRD_PARTY_NOTICES.md']:
                assert archive.read(f'Klyndb.app/Contents/Resources/{notice}') == Path(notice).read_bytes()
            assert 'Klyndb.app/Contents/MacOS/klyndb' in archive.namelist()
            assert archive.read('Klyndb.app/Contents/Resources/resources/THIRD_PARTY_LICENSES.txt') == (app / 'Contents/Resources/resources/THIRD_PARTY_LICENSES.txt').read_bytes()
    artifacts[path.name] = {'bytes': path.stat().st_size, 'sha256': hashlib.file_digest(path.open('rb'), 'sha256').hexdigest()}
manifest = {'source_commit': sys.argv[3], 'version': info['CFBundleShortVersionString'], 'architecture': sys.argv[4], 'builder_macos': platform.mac_ver()[0], 'signing': 'ad-hoc; not notarized', 'native_ui_acceptance': 'pending', 'artifacts': artifacts}
(output / 'BUILD_INFO.json').write_text(json.dumps(manifest, indent=2) + '\n')
(output / 'SHA256SUMS').write_text(''.join(f"{data['sha256']}  {name}\n" for name, data in artifacts.items()))
print(json.dumps(manifest, indent=2))
PY
printf 'Candidate directory: %s\n' "$output"
