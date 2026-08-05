import argparse
import json
import os
import subprocess
import time
import urllib.request
from pathlib import Path


ROOT = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace")
PORTABLE = ROOT / "savelink-app" / "src-tauri" / "target" / "release" / "bundle" / "portable" / "SaveLink"
EXE = PORTABLE / "SaveLink.exe"

parser = argparse.ArgumentParser()
parser.add_argument("profile")
parser.add_argument("port", type=int)
args = parser.parse_args()

profile = Path(args.profile).resolve()
profile.mkdir(parents=True, exist_ok=True)
env = os.environ.copy()
env["SAVELINK_TEST_DATA_DIR"] = str(profile)
env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = f"--remote-debugging-port={args.port}"
process = subprocess.Popen(
    [str(EXE)],
    cwd=str(PORTABLE),
    env=env,
    stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
    creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS,
)

deadline = time.time() + 30
targets = None
while time.time() < deadline:
    try:
        with urllib.request.urlopen(f"http://127.0.0.1:{args.port}/json", timeout=1) as response:
            targets = json.load(response)
            break
    except Exception:
        time.sleep(0.25)
if not targets:
    process.terminate()
    raise RuntimeError("WebView2 debug target did not appear")
print(json.dumps({"pid": process.pid, "port": args.port, "targets": len(targets)}, ensure_ascii=False))
