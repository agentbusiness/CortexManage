# Electron branded as CortexManage for macOS, shared by scripts/build-app.sh and the release scripts.
# Source it, then: electron_mac_branded <electron npm package> <destination package folder> <icns>
#
# The destination is a package folder like node_modules/electron: dist/CortexManage.app (renamed,
# its executable, name and icon CortexManage, so the Dock, Force Quit and Activity Monitor show
# CortexManage; ad-hoc signed), path.txt (the executable inside dist/) and version.

# This branding's number in the release catalog, which names an Electron "<version>-<number>". An
# installed updater keeps one folder per name and never replaces it, and a release reuses the archive
# of a name already published, so raise this whenever what electron_mac_branded makes changes.
# 2: the first archives published kept "Electron Helper".
# 3: Resources/app, so a start from a kept Dock icon opens the app instead of Electron's default window.
ELECTRON_BRANDING=3

electron_mac_branded() {
  local src="$1" dst="$2" icns="$3"
  rm -rf "$dst"
  mkdir -p "$dst/dist"
  local source_app
  source_app="$(ls -d "$src"/dist/*.app | head -1)"
  ditto "$source_app" "$dst/dist/CortexManage.app"
  local macos="$dst/dist/CortexManage.app/Contents/MacOS"
  [ -f "$macos/Electron" ] && mv "$macos/Electron" "$macos/CortexManage"
  # The helpers too, so Activity Monitor shows "CortexManage Helper (Renderer)", not "Electron Helper":
  # Electron looks for "Electron Helper.app" first, then "<CFBundleName> Helper.app".
  local frameworks="$dst/dist/CortexManage.app/Contents/Frameworks" helper base name
  for helper in "$frameworks"/Electron\ Helper*.app; do
    [ -d "$helper" ] || continue
    base="${helper##*/}"
    base="${base%.app}"
    name="CortexManage${base#Electron}"
    mv "$helper/Contents/MacOS/$base" "$helper/Contents/MacOS/$name"
    /usr/libexec/PlistBuddy -c "Set :CFBundleName $name" "$helper/Contents/Info.plist"
    mv "$helper" "$frameworks/$name.app"
  done
  local plist="$dst/dist/CortexManage.app/Contents/Info.plist" key
  for key in CFBundleName CFBundleDisplayName CFBundleExecutable; do
    /usr/libexec/PlistBuddy -c "Set :$key CortexManage" "$plist" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :$key string CortexManage" "$plist"
  done
  # Electron runs Resources/app when it exists. Started by the frame (CORTEX_FRAME_URL set), it loads
  # the shell it was given the way Electron's default app does. Started any other way, as from an icon
  # kept in the Dock (which keeps this bundle, not the app), it opens the app and quits.
  local launcher="$dst/dist/CortexManage.app/Contents/Resources/app" bundle_id
  bundle_id="$(/usr/libexec/PlistBuddy -c "Print :CFBundleIdentifier" "$repo/scripts/app/Info.plist")" || return 1
  mkdir -p "$launcher"
  printf '{"name":"cortexmanage-launcher","main":"main.js","type":"module"}\n' > "$launcher/package.json"
  sed "s/@BUNDLE_ID@/$bundle_id/" > "$launcher/main.js" <<'JS'
import { app } from 'electron/main';
import { spawn } from 'node:child_process';
import fs from 'node:fs';
import { Module } from 'node:module';
import path from 'node:path';
import url from 'node:url';

const shell = process.argv.slice(1).find((arg) => !arg.startsWith('-'));
if (process.env.CORTEX_FRAME_URL && shell) {
  Object.defineProperty(process, 'defaultApp', { configurable: false, enumerable: true, value: true });
  const packagePath = path.resolve(shell);
  const packageJson = JSON.parse(fs.readFileSync(path.join(packagePath, 'package.json'), 'utf8'));
  if (packageJson.version) app.setVersion(packageJson.version);
  app.name = packageJson.productName || packageJson.name || app.name;
  app.setAppPath(packagePath);
  await import(url.pathToFileURL(Module._resolveFilename(packagePath, null, true)).toString());
} else {
  spawn('/usr/bin/open', ['-b', '@BUNDLE_ID@'], { detached: true, stdio: 'ignore' }).unref();
  app.quit();
}
JS
  cp "$icns" "$dst/dist/CortexManage.app/Contents/Resources/CortexManage.icns"
  /usr/libexec/PlistBuddy -c "Set :CFBundleIconFile CortexManage.icns" "$plist" 2>/dev/null || /usr/libexec/PlistBuddy -c "Add :CFBundleIconFile string CortexManage.icns" "$plist"
  codesign --force --deep --sign - "$dst/dist/CortexManage.app" >/dev/null 2>&1
  printf '%s' "CortexManage.app/Contents/MacOS/CortexManage" > "$dst/path.txt"
  /usr/bin/python3 -c 'import json,sys; print(json.load(open(sys.argv[1]))["version"], end="")' "$src/package.json" > "$dst/version"
}
