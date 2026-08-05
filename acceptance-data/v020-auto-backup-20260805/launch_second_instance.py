import os
import subprocess
import time
from pathlib import Path


root = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace")
run_root = root / "acceptance-data" / "v020-auto-backup-20260805"
portable = root / "savelink-app" / "src-tauri" / "target" / "release" / "bundle" / "portable" / "SaveLink"
exe = portable / "SaveLink.exe"
env = os.environ.copy()
env["SAVELINK_TEST_DATA_DIR"] = str(run_root / "profile")
second = subprocess.Popen(
    [str(exe)],
    cwd=str(portable),
    env=env,
    stdin=subprocess.DEVNULL,
    stdout=subprocess.DEVNULL,
    stderr=subprocess.DEVNULL,
)
try:
    return_code = second.wait(timeout=8)
except subprocess.TimeoutExpired:
    return_code = None
print({"second_pid": second.pid, "second_return_code": return_code})
if return_code is None:
    second.terminate()
    raise RuntimeError("second SaveLink instance stayed alive")
