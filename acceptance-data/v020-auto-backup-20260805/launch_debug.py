import json
import os
import subprocess
import time
import urllib.request
from pathlib import Path


ROOT = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace")
RUN_ROOT = ROOT / "acceptance-data" / "v020-auto-backup-20260805"
PORTABLE_DIR = (
    ROOT
    / "savelink-app"
    / "src-tauri"
    / "target"
    / "release"
    / "bundle"
    / "portable"
    / "SaveLink"
)
EXE = PORTABLE_DIR / "SaveLink.exe"

env = os.environ.copy()
env["SAVELINK_TEST_DATA_DIR"] = str(RUN_ROOT / "profile")
env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = "--remote-debugging-port=9229"
process = subprocess.Popen(
    [str(EXE)],
    cwd=str(PORTABLE_DIR),
    env=env,
    stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
    creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS,
)

deadline = time.time() + 20
targets = None
while time.time() < deadline:
    try:
        with urllib.request.urlopen("http://127.0.0.1:9229/json", timeout=1) as response:
            targets = json.load(response)
            break
    except Exception:
        time.sleep(0.25)

if not targets:
    process.terminate()
    raise RuntimeError("WebView2 debug target did not appear")

print(
    json.dumps(
        {
            "pid": process.pid,
            "target_count": len(targets),
            "title": targets[0].get("title"),
            "url": targets[0].get("url"),
        },
        ensure_ascii=False,
    )
)
