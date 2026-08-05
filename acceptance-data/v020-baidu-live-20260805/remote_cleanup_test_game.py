import json
import urllib.parse
import urllib.request
from pathlib import Path


RUN_ROOT = Path(r"C:\Users\daiqiang\door\project_workspace\save_link_workspace\acceptance-data\v020-baidu-live-20260805")
TOKEN_FILE = RUN_ROOT / "profile-a" / "credentials" / "baidu-oauth.json"
GAME_ID = "game_1785936338508951200_0"
GAMES_DIR = "/apps/savelink/v1/games"
GAME_DIR = f"{GAMES_DIR}/{GAME_ID}"
USER_AGENT = "pan.baidu.com"


def request_json(request):
    with urllib.request.urlopen(request, timeout=120) as response:
        return json.loads(response.read().decode("utf-8"))


token = json.loads(TOKEN_FILE.read_text(encoding="utf-8-sig"))["access_token"]
delete_query = urllib.parse.urlencode(
    {"method": "filemanager", "access_token": token, "opera": "delete"}
)
delete_body = urllib.parse.urlencode(
    {"async": "0", "filelist": json.dumps([{"path": GAME_DIR}])}
).encode("utf-8")
delete_request = urllib.request.Request(
    f"https://pan.baidu.com/rest/2.0/xpan/file?{delete_query}",
    data=delete_body,
    headers={"User-Agent": USER_AGENT},
    method="POST",
)
delete_result = request_json(delete_request)
if delete_result.get("errno", 0) != 0:
    raise RuntimeError(f"Baidu delete failed: errno={delete_result.get('errno')}")
for item in delete_result.get("info", []):
    if item.get("errno", 0) != 0:
        raise RuntimeError(f"Baidu item delete failed: errno={item.get('errno')}")

list_query = urllib.parse.urlencode(
    {
        "method": "list",
        "access_token": token,
        "dir": GAMES_DIR,
        "order": "name",
        "start": "0",
        "limit": "1000",
        "web": "1",
        "folder": "0",
        "desc": "0",
    }
)
list_request = urllib.request.Request(
    f"https://pan.baidu.com/rest/2.0/xpan/file?{list_query}",
    headers={"User-Agent": USER_AGENT},
)
listing = request_json(list_request)
if listing.get("errno", 0) != 0:
    raise RuntimeError(f"Baidu parent listing failed: errno={listing.get('errno')}")
remaining_game_ids = sorted(item["server_filename"] for item in listing.get("list", []))
assert GAME_ID not in remaining_game_ids, "test game directory still exists remotely"

result = {
    "passed": True,
    "deleted_path": GAME_DIR,
    "verified_via_parent_listing": True,
    "test_game_present_after_delete": False,
}
(RUN_ROOT / "remote-cleanup-result.json").write_text(
    json.dumps(result, ensure_ascii=False, indent=2), encoding="utf-8"
)
print(json.dumps(result, ensure_ascii=False))
