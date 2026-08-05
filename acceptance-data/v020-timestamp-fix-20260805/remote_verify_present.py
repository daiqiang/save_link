import hashlib
import json
import sqlite3
import time
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path


RUN_ROOT = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace\acceptance-data\v020-timestamp-fix-20260805")
PROFILE = RUN_ROOT / "profile-a"
TOKEN_FILE = PROFILE / "credentials" / "baidu-oauth.json"
USER_AGENT = "pan.baidu.com"


def read_response(request, attempts=3):
    last_error = None
    for attempt in range(attempts):
        try:
            with urllib.request.urlopen(request, timeout=120) as response:
                return response.read()
        except Exception as error:
            last_error = error
            if attempt + 1 < attempts:
                time.sleep(attempt + 1)
    raise last_error


def get_json(url, params):
    request = urllib.request.Request(
        f"{url}?{urllib.parse.urlencode(params)}", headers={"User-Agent": USER_AGENT}
    )
    data = json.loads(read_response(request).decode("utf-8"))
    if data.get("errno", 0) != 0:
        raise RuntimeError(f"Baidu API error: errno={data.get('errno')}")
    return data


def download(entry, token):
    metas = get_json(
        "https://pan.baidu.com/rest/2.0/xpan/multimedia",
        {"method": "filemetas", "access_token": token, "fsids": json.dumps([entry["fs_id"]]), "dlink": "1"},
    )
    dlink = metas["list"][0]["dlink"]
    separator = "&" if "?" in dlink else "?"
    request = urllib.request.Request(
        f"{dlink}{separator}{urllib.parse.urlencode({'access_token': token})}",
        headers={"User-Agent": USER_AGENT},
    )
    payload = read_response(request)
    assert len(payload) == entry["size"]
    return payload


token = json.loads(TOKEN_FILE.read_text(encoding="utf-8-sig"))["access_token"]
with sqlite3.connect(PROFILE / "savelink.db") as connection:
    game_id, game_name = connection.execute("select id, name from games").fetchone()
    snapshot = connection.execute(
        "select id, created_at, reason, file_count, total_size, content_hash from snapshots"
    ).fetchone()
    cloud = connection.execute(
        "select sync_status, archive_size, archive_sha256 from cloud_snapshot_sync where snapshot_id=?",
        (snapshot[0],),
    ).fetchone()

snapshot_id, created_at, reason, file_count, total_size, content_hash = snapshot
assert created_at.endswith("Z") and "T" in created_at, created_at
assert cloud[0] == "uploaded", cloud
remote_dir = f"/apps/savelink/v1/games/{game_id}/snapshots"
listing = get_json(
    "https://pan.baidu.com/rest/2.0/xpan/file",
    {
        "method": "list", "access_token": token, "dir": remote_dir, "order": "name",
        "start": "0", "limit": "1000", "web": "1", "folder": "0", "desc": "0",
    },
)
entries = {item["server_filename"]: item for item in listing["list"]}
assert sorted(entries) == [f"{snapshot_id}.ok", f"{snapshot_id}.zip"], sorted(entries)
zip_bytes = download(entries[f"{snapshot_id}.zip"], token)
ok_bytes = download(entries[f"{snapshot_id}.ok"], token)
commit = json.loads(ok_bytes.decode("utf-8"))
zip_sha256 = hashlib.sha256(zip_bytes).hexdigest()
assert commit["snapshot_id"] == snapshot_id
assert commit["cloud_game_id"] == game_id
assert commit["created_at"] == created_at
assert commit["reason"] == reason
assert commit["file_count"] == file_count
assert commit["total_size"] == total_size
assert commit["content_hash"]["value"] == content_hash
assert commit["archive"]["size"] == len(zip_bytes) == cloud[1]
assert commit["archive"]["sha256"] == zip_sha256 == cloud[2]
zip_path = RUN_ROOT / "real-cloud-baseline.zip"
zip_path.write_bytes(zip_bytes)
with zipfile.ZipFile(zip_path) as archive:
    files = [item for item in archive.infolist() if not item.is_dir()]
    assert len(files) == file_count
    assert sum(item.file_size for item in files) == total_size

result = {
    "passed": True,
    "game_id": game_id,
    "game_name": game_name,
    "snapshot_id": snapshot_id,
    "created_at": created_at,
    "remote_dir": remote_dir,
    "remote_names": sorted(entries),
    "file_count": file_count,
    "total_size": total_size,
    "zip_size": len(zip_bytes),
    "zip_sha256": zip_sha256,
}
(RUN_ROOT / "real-cloud-baseline.json").write_text(
    json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8"
)
print(json.dumps(result, ensure_ascii=False))
