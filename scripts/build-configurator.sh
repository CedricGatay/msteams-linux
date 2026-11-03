#!/usr/bin/env bash
# Developer packaging only. The exported configurator uses this prebuilt
# runtime; end users do not need Node, Rust, Python or the source checkout.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PROFILE=debug
if [[ "${1:-}" == --release ]]; then PROFILE=release; shift; fi
if [[ $# != 0 || "$(uname -s)" != Darwin ]]; then
  echo "Usage (macOS): ./scripts/build-configurator.sh [--release]" >&2; exit 2
fi
cd "$ROOT"
if [[ "$PROFILE" == debug ]]; then npm run tauri -- build --debug --bundles app
else npm run tauri -- build --bundles app; fi
python3 - "$ROOT" "$PROFILE" <<'PY'
import json,pathlib,plistlib,shutil,subprocess,sys
root=pathlib.Path(sys.argv[1]); profile=sys.argv[2]
parent=root/'src-tauri/target'/profile/'bundle/macos'
runtime=parent/'Microsoft Teams.app'; output=parent/'Teams Configurator.app'
if (runtime/'Contents/Resources/runtime-config.json').exists():
 raise SystemExit('Refusing to package a personalized runtime as a public template')
commands=subprocess.check_output(['ps','-axo','command='],text=True).splitlines()
if any(line.strip().startswith(str(output)+'/Contents/MacOS/') for line in commands):
 raise SystemExit('Quit the configurator before rebuilding it')
if output.exists():shutil.rmtree(output)
shutil.copytree(runtime,output)
resources=output/'Contents/Resources'; resources.mkdir(exist_ok=True)
shutil.copytree(runtime,resources/'RuntimeTemplate.app')
(resources/'configurator-mode.json').write_text(json.dumps({'mode':'configurator','version':1})+'\n')
path=output/'Contents/Info.plist'
info=plistlib.loads(path.read_bytes());info.update(CFBundleName='Teams Configurator',CFBundleDisplayName='Teams Configurator',CFBundleIdentifier='com.github.msteams.configurator')
path.write_bytes(plistlib.dumps(info))
subprocess.run(['/usr/bin/codesign','--force','--sign','-','--identifier',info['CFBundleIdentifier'],'--options','runtime','--timestamp=none','--entitlements',str(root/'src-tauri/Entitlements.plist'),str(output)],check=True)
subprocess.run(['/usr/bin/codesign','--verify','--deep','--strict',str(output)],check=True)
print('Configurator ready:',output)
PY
