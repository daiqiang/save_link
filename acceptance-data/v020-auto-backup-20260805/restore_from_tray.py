import ctypes
import time
from pathlib import Path

from pywinauto import Desktop


root = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace\acceptance-data\v020-auto-backup-20260805")
window = Desktop(backend="uia").window(title="SaveLink", visible_only=False)
handle = window.handle
ctypes.windll.user32.ShowWindow(handle, 9)
ctypes.windll.user32.SetForegroundWindow(handle)
time.sleep(2)
window = Desktop(backend="uia").window(title="SaveLink", visible_only=True)
window.capture_as_image().save(root / "10-restored-from-tray.png")
print({"handle": handle, "visible": window.is_visible(), "title": window.window_text()})
