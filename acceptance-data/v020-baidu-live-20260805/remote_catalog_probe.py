import hashlib
import json
import sqlite3
import time
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path


RUN_ROOT = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace\acceptance-data\v020-baidu-live-20260805")
PROFILE = RUN_ROOT / "profile-a"
TOKEN_FILE = PROFILE / "credentials" / "baidu-oauth.json"
DB_FILE = PROFILE / "savelink.db"
PROOF_DIR = RUN_ROOT / "remote-proof-all"
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
                time.sleep(1 + attempt)
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
    dlink = next(item["dlink"] for item in metas["list"] if item["fs_id"] == entry["fs_id"])
    separator = "&" if "?" in dlink else "?"
    request = urllib.request.Request(
        f"{dlink}{separator}{urllib.parse.urlencode({'access_token': token})}",
        headers={"User-Agent": USER_AGENT},
    )
    payload = read_response(request)
    if len(payload) != entry["size"]:
        raise RuntimeError(f"remote size mismatch for {entry['server_filename']}")
    return payload


token = json.loads(TOKEN_FILE.read_text(encoding="utf-8-sig"))["access_token"]
with sqlite3.connect(DB_FILE) as connection:
    game_id = connection.execute("select id from games").fetchone()[0]
    records = connection.execute(
        "select s.id, s.reason, s.file_count, s.total_size, "
        "c.archive_size, c.archive_sha256, c.content_hash "
        "from snapshots s join cloud_snapshot_sync c on c.snapshot_id=s.id "
        "where c.sync_status='uploaded' order by s.created_at, s.id"
    ).fetchall()

remote_dir = f"/apps/savelink/v1/games/{game_id}/snapshots"
listing = get_json(
    "https://pan.baidu.com/rest/2.0/xpan/file",
    {
        "method": "list", "access_token": token, "dir": remote_dir, "order": "name",
        "start": "0", "limit": "1000", "web": "1", "folder": "0", "desc": "0",
    },
)
entries = {item["server_filename"]: item for item in listing["list"]}
verified = []
PROOF_DIR.mkdir(parents=True, exist_ok=True)
for snapshot_id, reason, file_count, total_size, archive_size, archive_sha256, content_hash in records:
    zip_name = f"{snapshot_id}.zip"
    ok_name = f"{snapshot_id}.ok"
    if zip_name not in entries or ok_name not in entries:
        raise RuntimeError(f"remote pair missing for {snapshot_id}")
    zip_bytes = download(entries[zip_name], token)
    ok_bytes = download(entries[ok_name], token)
    commit = json.loads(ok_bytes.decode("utf-8"))
    zip_sha256 = hashlib.sha256(zip_bytes).hexdigest()
    assert commit["snapshot_id"] == snapshot_id
    assert commit["cloud_game_id"] == game_id
    assert commit["reason"] == reason
    assert commit["file_count"] == file_count
    assert commit["total_size"] == total_size
    assert commit["archive"]["size"] == len(zip_bytes) == archive_size
    assert commit["archive"]["sha256"] == zip_sha256 == archive_sha256
    assert commit["content_hash"]["value"] == content_hash
    snapshot_dir = PROOF_DIR / snapshot_id
    snapshot_dir.mkdir(parents=True, exist_ok=True)
    zip_path = snapshot_dir / zip_name
    zip_path.write_bytes(zip_bytes)
    (snapshot_dir / ok_name).write_bytes(ok_bytes)
    with zipfile.ZipFile(zip_path) as archive:
        files = [item for item in archive.infolist() if not item.is_dir()]
        assert len(files) == file_count
        assert sum(item.file_size for item in files) == total_size
    verified.append(
        {
            "snapshot_id": snapshot_id,
            "reason": reason,
            "file_count": file_count,
            "total_size": total_size,
            "zip_size": len(zip_bytes),
            "zip_sha256": zip_sha256,
        }
    )

result = {
    "passed": True,
    "game_id": game_id,
    "remote_dir": remote_dir,
    "verified": verified,
    "remote_names": sorted(entries),
}
(RUN_ROOT / "remote-catalog-result.json").write_text(
    json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8"
)
print(json.dumps(result, ensure_ascii=False))
