const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-baidu-live-20260805";
const saveA = path.join(runRoot, "save-a");
const restoreB = path.join(runRoot, "restore-b");
const gameName = "SaveLink百度实测-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

(async () => {
  const result = { phase: "C-01-profile-b-discover-download-restore", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9231");
    const page = browser.contexts()[0].pages()[0];
    await page.getByText("设备 B 隔离测试", { exact: true }).waitFor();
    await page.getByRole("button", { name: "云端存档", exact: true }).click();
    const cloudModal = page.locator(".cloud-modal");
    await cloudModal.getByRole("heading", { name: "云端存档", exact: true }).waitFor();
    const testGame = cloudModal.locator(".cloud-game").filter({ hasText: "20260805" });
    await testGame.waitFor({ timeout: 30000 });
    await testGame.getByRole("button", { name: "下载", exact: true }).click();
    await page.getByText("云端快照已下载到本机仓库", { exact: true }).waitFor({ timeout: 120000 });
    await testGame.getByText("已在本机", { exact: true }).waitFor();
    result.observations.push("fresh profile discovered and downloaded the uploaded snapshot");
    await page.screenshot({ path: path.join(runRoot, "03-profile-b-downloaded.png") });
    await cloudModal.getByTitle("关闭").click();

    await page.getByRole("heading", { name: gameName, exact: true }).waitFor();
    await page.getByText("尚未绑定本机存档目录", { exact: true }).first().waitFor();
    await page.getByRole("button", { name: "绑定存档目录", exact: true }).click();
    const bindModal = page.locator(".modal").filter({ has: page.getByRole("heading", { name: "绑定存档目录", exact: true }) });
    await bindModal.locator("input").fill(restoreB);
    await bindModal.getByRole("button", { name: "测试读取", exact: true }).click();
    await bindModal.getByText("目录可读取，当前为空", { exact: true }).waitFor();
    await bindModal.getByRole("button", { name: "确认绑定", exact: true }).click();
    await page.getByText("本机存档目录已绑定", { exact: true }).waitFor();

    const snapshot = page.locator(".snap").first();
    await snapshot.getByRole("button", { name: "恢复", exact: true }).click();
    await page.getByRole("button", { name: "恢复这个版本", exact: true }).click();
    await page.getByRole("heading", { name: "恢复完成", exact: true }).waitFor({ timeout: 30000 });
    await page.screenshot({ path: path.join(runRoot, "04-profile-b-restored.png") });
    await page.getByRole("button", { name: "回到时间线", exact: true }).click();

    const names = fs.readdirSync(restoreB).sort();
    assert(names.join(",") === "cloud-test.sav,config.ini", `restored files mismatch: ${names}`);
    for (const name of names) {
      assert(sha256(path.join(restoreB, name)) === sha256(path.join(saveA, name)), `restored hash mismatch: ${name}`);
    }
    result.observations.push("downloaded snapshot restored to an empty bound directory with identical file hashes");
    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-profile-b-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-profile-b-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
