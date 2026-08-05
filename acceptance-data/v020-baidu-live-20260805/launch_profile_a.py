import json
import os
import subprocess
import time
import urllib.request
from pathlib import Path


root = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace")
run_root = root / "acceptance-data" / "v020-baidu-live-20260805"
portable = root / "savelink-app" / "src-tauri" / "target" / "release" / "bundle" / "portable" / "SaveLink"
exe = portable / "SaveLink.exe"
env = os.environ.copy()
env["SAVELINK_TEST_DATA_DIR"] = str(run_root / "profile-a")
env["WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS"] = "--remote-debugging-port=9230"
process = subprocess.Popen(
    [str(exe)],
    cwd=str(portable),
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
        with urllib.request.urlopen("http://127.0.0.1:9230/json", timeout=1) as response:
            targets = json.load(response)
            break
    except Exception:
        time.sleep(0.25)
if not targets:
    process.terminate()
    raise RuntimeError("profile A WebView2 debug target did not appear")
print(json.dumps({"pid": process.pid, "targets": len(targets), "title": targets[0].get("title")}, ensure_ascii=False))
