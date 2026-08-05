import json
import re
import sqlite3
import urllib.parse
import urllib.request
from pathlib import Path


RUN_ROOT = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace\acceptance-data\v020-timestamp-fix-20260805")
PROFILE = RUN_ROOT / "profile-b"
TOKEN_FILE = PROFILE / "credentials" / "baidu-oauth.json"
BASELINE = json.loads((RUN_ROOT / "real-cloud-baseline.json").read_text(encoding="utf-8"))
USER_AGENT = "pan.baidu.com"


def request_json(request):
    with urllib.request.urlopen(request, timeout=120) as response:
        data = json.loads(response.read().decode("utf-8"))
    if data.get("errno", 0) != 0:
        raise RuntimeError(f"Baidu API error: errno={data.get('errno')}")
    return data


def list_dir(path, token):
    query = urllib.parse.urlencode(
        {
            "method": "list", "access_token": token, "dir": path, "order": "name",
            "start": "0", "limit": "1000", "web": "1", "folder": "0", "desc": "0",
        }
    )
    return request_json(
        urllib.request.Request(
            f"https://pan.baidu.com/rest/2.0/xpan/file?{query}",
            headers={"User-Agent": USER_AGENT},
        )
    )


token = json.loads(TOKEN_FILE.read_text(encoding="utf-8-sig"))["access_token"]
with sqlite3.connect(PROFILE / "savelink.db") as connection:
    rows = connection.execute(
        "select id, created_at, reason, status from snapshots order by created_at desc, rowid desc"
    ).fetchall()
    cloud_rows = connection.execute(
        "select cloud_game_id, snapshot_id, sync_status from cloud_snapshot_sync"
    ).fetchall()
    auto_enabled = connection.execute(
        "select value from app_settings where key='auto_backup_enabled'"
    ).fetchone()[0]

assert len(rows) == 30, len(rows)
assert BASELINE["snapshot_id"] not in {row[0] for row in rows}
assert all(re.fullmatch(r"\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z", row[1]) for row in rows)
assert all(row[1] > BASELINE["created_at"] for row in rows)
assert all(row[2] == "manual" and row[3] == "complete" for row in rows)
test_cloud_rows = [row for row in cloud_rows if row[0] == BASELINE["game_id"]]
unrelated_cloud_rows = [row for row in cloud_rows if row[0] != BASELINE["game_id"]]
assert test_cloud_rows == [], test_cloud_rows
assert auto_enabled == "false"

snapshots_dir = BASELINE["remote_dir"]
snapshot_listing = list_dir(snapshots_dir, token)
assert snapshot_listing["list"] == [], snapshot_listing["list"]

game_dir = snapshots_dir.rsplit("/snapshots", 1)[0]
delete_query = urllib.parse.urlencode(
    {"method": "filemanager", "access_token": token, "opera": "delete"}
)
delete_body = urllib.parse.urlencode(
    {"async": "0", "filelist": json.dumps([{"path": game_dir}])}
).encode("utf-8")
delete_result = request_json(
    urllib.request.Request(
        f"https://pan.baidu.com/rest/2.0/xpan/file?{delete_query}",
        data=delete_body,
        headers={"User-Agent": USER_AGENT},
        method="POST",
    )
)
for item in delete_result.get("info", []):
    if item.get("errno", 0) != 0:
        raise RuntimeError(f"Baidu item delete failed: errno={item.get('errno')}")

games_dir = game_dir.rsplit("/", 1)[0]
remaining = [item["server_filename"] for item in list_dir(games_dir, token)["list"]]
assert BASELINE["game_id"] not in remaining

result = {
    "passed": True,
    "local_snapshot_count": len(rows),
    "all_local_timestamps_utc_rfc3339": True,
    "all_remaining_snapshots_newer_than_cloud_baseline": True,
    "deleted_cloud_snapshot_id": BASELINE["snapshot_id"],
    "remote_snapshot_names_after_retention": [],
    "test_game_cloud_rows_after_retention": 0,
    "unrelated_remote_only_rows_preserved": len(unrelated_cloud_rows),
    "auto_backup_enabled_after_test": False,
    "deleted_remote_game_dir": game_dir,
    "verified_absent_from_parent": True,
}
(RUN_ROOT / "real-cloud-cleanup-result.json").write_text(
    json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8"
)
print(json.dumps(result, ensure_ascii=False))
