import json
import sqlite3
import urllib.parse
import urllib.request
from pathlib import Path


RUN_ROOT = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace\acceptance-data\v020-baidu-live-20260805")
PROFILE = RUN_ROOT / "profile-a"
TOKEN_FILE = PROFILE / "credentials" / "baidu-oauth.json"
DB_FILE = PROFILE / "savelink.db"
USER_AGENT = "pan.baidu.com"
MANUAL_ID = "snap_1785936338554505200_1"
AUTO_ID = "snap_1785936680866133900_0"


def get_json(url, params):
    request = urllib.request.Request(
        f"{url}?{urllib.parse.urlencode(params)}", headers={"User-Agent": USER_AGENT}
    )
    with urllib.request.urlopen(request, timeout=120) as response:
        data = json.loads(response.read().decode("utf-8"))
    if data.get("errno", 0) != 0:
        raise RuntimeError(f"Baidu API error: errno={data.get('errno')}")
    return data


token = json.loads(TOKEN_FILE.read_text(encoding="utf-8-sig"))["access_token"]
with sqlite3.connect(DB_FILE) as connection:
    game_id = connection.execute("select id from games").fetchone()[0]
    snapshot_count = connection.execute("select count(*) from snapshots").fetchone()[0]
    local_ids = {row[0] for row in connection.execute("select id from snapshots")}
    cloud_rows = connection.execute(
        "select snapshot_id, sync_status from cloud_snapshot_sync order by snapshot_id"
    ).fetchall()

remote_dir = f"/apps/savelink/v1/games/{game_id}/snapshots"
listing = get_json(
    "https://pan.baidu.com/rest/2.0/xpan/file",
    {
        "method": "list",
        "access_token": token,
        "dir": remote_dir,
        "order": "name",
        "start": "0",
        "limit": "1000",
        "web": "1",
        "folder": "0",
        "desc": "0",
    },
)
remote_names = sorted(item["server_filename"] for item in listing["list"])
assert snapshot_count == 30, f"expected 30 local snapshots, got {snapshot_count}"
assert MANUAL_ID not in local_ids, "oldest manual snapshot still exists locally"
assert f"{MANUAL_ID}.ok" not in remote_names and f"{MANUAL_ID}.zip" not in remote_names

stage = "after-manual-delete" if AUTO_ID in local_ids else "after-auto-delete"
if stage == "after-manual-delete":
    assert cloud_rows == [(AUTO_ID, "uploaded")], cloud_rows
    assert remote_names == [f"{AUTO_ID}.ok", f"{AUTO_ID}.zip"], remote_names
else:
    assert cloud_rows == [], cloud_rows
    assert remote_names == [], remote_names

result = {
    "passed": True,
    "stage": stage,
    "game_id": game_id,
    "snapshot_count": snapshot_count,
    "cloud_rows": cloud_rows,
    "remote_names": remote_names,
}
(RUN_ROOT / f"remote-retention-{stage}-result.json").write_text(
    json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8"
)
print(json.dumps(result, ensure_ascii=False))
