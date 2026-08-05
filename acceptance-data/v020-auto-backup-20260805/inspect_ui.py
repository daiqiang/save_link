import os
import subprocess
import time
from pathlib import Path

from pywinauto import Desktop


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


deadline = time.time() + 15
window = None
process = None
existing = Desktop(backend="uia").windows(title_re=r".*SaveLink.*", visible_only=True)
if existing:
    window = existing[0]
else:
    env = os.environ.copy()
    env["SAVELINK_TEST_DATA_DIR"] = str(RUN_ROOT / "profile")
    process = subprocess.Popen(
        [str(EXE)],
        cwd=str(PORTABLE_DIR),
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
        creationflags=subprocess.CREATE_NEW_PROCESS_GROUP | subprocess.DETACHED_PROCESS,
    )
while time.time() < deadline:
    if window is not None:
        break
    candidates = Desktop(backend="uia").windows(title_re=r".*SaveLink.*", visible_only=True)
    if candidates:
        window = candidates[0]
        break
    time.sleep(0.25)

if window is None:
    if process is not None:
        process.terminate()
    raise RuntimeError("SaveLink window did not appear within 15 seconds")

window.set_focus()
time.sleep(1)
window.capture_as_image().save(RUN_ROOT / "01-fresh-start.png")

print(f"PID={process.pid if process is not None else 'existing'}")
print(f"TITLE={window.window_text()}")
print(f"RECT={window.rectangle()}")
for index, control in enumerate(window.descendants()):
    if index >= 250:
        print("...TRUNCATED...")
        break
    info = control.element_info
    print(
        f"{index:03d} type={info.control_type!r} "
        f"name={info.name!r} auto_id={info.automation_id!r} "
        f"class={info.class_name!r} rect={info.rectangle}"
    )
