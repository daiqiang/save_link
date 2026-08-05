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
PROOF_DIR = RUN_ROOT / "remote-proof"
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
    query = urllib.parse.urlencode(params)
    request = urllib.request.Request(f"{url}?{query}", headers={"User-Agent": USER_AGENT})
    data = json.loads(read_response(request).decode("utf-8"))
    if data.get("errno", 0) != 0:
        raise RuntimeError(f"Baidu API error: errno={data.get('errno')}")
    return data


def download(entry, token, destination):
    metas = get_json(
        "https://pan.baidu.com/rest/2.0/xpan/multimedia",
        {
            "method": "filemetas",
            "access_token": token,
            "fsids": json.dumps([entry["fs_id"]]),
            "dlink": "1",
        },
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
    destination.write_bytes(payload)
    return payload


token_data = json.loads(TOKEN_FILE.read_text(encoding="utf-8-sig"))
token = token_data["access_token"]
with sqlite3.connect(DB_FILE) as connection:
    game_id, game_name = connection.execute("select id, name from games").fetchone()
    snapshot_id = connection.execute("select id from snapshots order by created_at limit 1").fetchone()[0]
    cloud = connection.execute(
        "select sync_status, archive_size, archive_sha256, content_hash "
        "from cloud_snapshot_sync where snapshot_id = ?",
        (snapshot_id,),
    ).fetchone()

if cloud is None or cloud[0] != "uploaded":
    raise RuntimeError("local cloud state is not uploaded")

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
entries = {item["server_filename"]: item for item in listing["list"]}
zip_name = f"{snapshot_id}.zip"
ok_name = f"{snapshot_id}.ok"
if zip_name not in entries or ok_name not in entries:
    raise RuntimeError("remote .zip/.ok pair is incomplete")

PROOF_DIR.mkdir(parents=True, exist_ok=True)
zip_bytes = download(entries[zip_name], token, PROOF_DIR / zip_name)
ok_bytes = download(entries[ok_name], token, PROOF_DIR / ok_name)
commit = json.loads(ok_bytes.decode("utf-8"))

zip_sha256 = hashlib.sha256(zip_bytes).hexdigest()
assert commit["snapshot_id"] == snapshot_id
assert commit["cloud_game_id"] == game_id
assert commit["archive"]["file_name"] == zip_name
assert commit["archive"]["size"] == len(zip_bytes) == cloud[1]
assert commit["archive"]["sha256"] == zip_sha256 == cloud[2]
assert commit["content_hash"]["value"] == cloud[3]

with zipfile.ZipFile(PROOF_DIR / zip_name) as archive:
    files = [item for item in archive.infolist() if not item.is_dir()]
    archive_names = sorted(item.filename for item in files)
    uncompressed_size = sum(item.file_size for item in files)

assert len(files) == commit["file_count"] == 2
assert uncompressed_size == commit["total_size"] == 46

result = {
    "passed": True,
    "game_id": game_id,
    "game_name": game_name,
    "snapshot_id": snapshot_id,
    "remote_dir": remote_dir,
    "remote_names": sorted(name for name in entries if name.startswith(snapshot_id)),
    "zip_size": len(zip_bytes),
    "zip_sha256": zip_sha256,
    "ok_size": len(ok_bytes),
    "archive_files": archive_names,
    "file_count": len(files),
    "uncompressed_size": uncompressed_size,
}
(RUN_ROOT / "remote-probe-result.json").write_text(
    json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8"
)
print(json.dumps(result, ensure_ascii=False))
