import time
from pathlib import Path

from pywinauto import Desktop


root = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace\acceptance-data\v020-auto-backup-20260805")
window = Desktop(backend="uia").window(title="SaveLink", visible_only=True)
window.child_window(title="关闭", control_type="Button").click_input()
time.sleep(2)
visible = Desktop(backend="uia").windows(title_re=r".*SaveLink.*", visible_only=True)
window.capture_as_image().save(root / "09-tray-hidden.png") if visible else None
print({"visible_windows_after_close": len(visible)})
