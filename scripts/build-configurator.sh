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
python3 - "$ROOT" "$PROFILE" <<'PY'
import pathlib,subprocess,sys
parent=pathlib.Path(sys.argv[1])/'src-tauri/target'/sys.argv[2]/'bundle/macos'
commands=subprocess.check_output(['ps','-axo','command='],text=True).splitlines()
for name in ['Microsoft Teams.app','Teams Configurator.app']:
 if any(line.strip().startswith(str(parent/name)+'/Contents/MacOS/') for line in commands):
  raise SystemExit('Quit the build-output app before rebuilding it')
PY
SIGNER="${MSTEAMS_SIGNING_IDENTITY:-${APPLE_SIGNING_IDENTITY:--}}"
export APPLE_SIGNING_IDENTITY="$SIGNER"
SIGNING_CONFIG="$(python3 - "$SIGNER" <<'PY'
import json,sys
print(json.dumps({'bundle': {'macOS': {'signingIdentity': sys.argv[1]}}}))
PY
)"
TAURI_ARGS=(build --bundles app --config "$SIGNING_CONFIG")
if [[ "$PROFILE" == debug ]]; then TAURI_ARGS+=(--debug); fi
npm run tauri -- "${TAURI_ARGS[@]}"
python3 - "$ROOT" "$PROFILE" "$SIGNER" <<'PY'
import json,pathlib,plistlib,shutil,subprocess,sys
root=pathlib.Path(sys.argv[1]); profile=sys.argv[2]; signer=sys.argv[3]
distribution=json.loads((root/'config/distribution.json').read_text())
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
info=plistlib.loads(path.read_bytes());info.update(CFBundleName='Teams Configurator',CFBundleDisplayName='Teams Configurator',CFBundleIdentifier=distribution['configurator_identifier'])
path.write_bytes(plistlib.dumps(info))
timestamp='--timestamp=none' if signer == '-' else '--timestamp'
subprocess.run(['/usr/bin/codesign','--force','--sign',signer,'--identifier',info['CFBundleIdentifier'],'--options','runtime',timestamp,'--entitlements',str(root/'src-tauri/Entitlements.plist'),str(output)],check=True)
subprocess.run(['/usr/bin/codesign','--verify','--deep','--strict',str(output)],check=True)
# The runtime is a build intermediate, not a second distributable. Its complete
# copy remains sealed inside the configurator for generating independent apps.
shutil.rmtree(runtime)
print('Configurator ready:',output)
PY
