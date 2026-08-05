const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";
const goodGame = path.join(runRoot, "saves", "good-game");
const brokenGame = path.join(runRoot, "saves", "broken-game");
const brokenMissing = path.join(runRoot, "saves", "broken-game.missing");
const tokenPath = path.join(runRoot, "profile", "credentials", "baidu-oauth.json");

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

async function setAutoBackup(page, enabled) {
  await page.getByTitle("设置").click();
  const autoSwitch = page.getByRole("switch", { name: "自动备份" });
  await autoSwitch.waitFor();
  const current = (await autoSwitch.getAttribute("aria-checked")) === "true";
  if (current !== enabled) {
    await autoSwitch.click();
    await page.locator(`[role="switch"][aria-checked="${enabled}"]`).waitFor();
  }
  await page.getByRole("button", { name: "完成", exact: true }).click();
}

async function addGame(page, name, savePath, expectedFiles) {
  await page.getByTitle("添加游戏").click();
  const modal = page.locator(".modal");
  await modal.getByRole("heading", { name: "添加游戏", exact: true }).waitFor();
  await modal.locator("input").nth(0).fill(name);
  await modal.locator("input").nth(1).fill(savePath);
  await modal.getByRole("button", { name: "测试读取", exact: true }).click();
  await modal.getByText(new RegExp(`已检测到：${expectedFiles} 个文件`)).waitFor();
  await modal.getByRole("button", { name: "保存并创建", exact: true }).click();
  await page.getByRole("heading", { name, exact: true }).waitFor();
}

(async () => {
  const result = { phase: "U-07-U-08-failure-isolation-no-auth", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "自动备份测试游戏", exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 4, "good game should start with four snapshots");
    assert(!fs.existsSync(tokenPath), "isolated profile unexpectedly contains a Baidu token");

    await setAutoBackup(page, false);
    await addGame(page, "自动备份故障游戏", brokenGame, 1);
    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.getByText("快照已创建", { exact: true }).waitFor();
    await page.locator(".snap").waitFor();
    assert((await page.locator(".snap").count()) === 1, "broken game baseline should contain one snapshot");

    await page.locator(".game-item").filter({ hasText: "自动备份测试游戏" }).click();
    await page.getByRole("heading", { name: "自动备份测试游戏", exact: true }).waitFor();
    fs.writeFileSync(path.join(goodGame, "slot1.sav"), "good-v5-failure-isolation\n", "utf8");
    fs.renameSync(brokenGame, brokenMissing);
    await setAutoBackup(page, true);

    await page.locator(".snap").nth(4).waitFor({ timeout: 15000 });
    assert((await page.locator(".snap").count()) === 5, "healthy game did not create an auto snapshot when another game failed");
    assert((await page.locator(".snap").filter({ hasText: "自动快照" }).count()) === 4, "healthy game auto reason count mismatch");
    result.observations.push("healthy game created an auto snapshot despite another missing directory");

    await page.locator(".game-item").filter({ hasText: "自动备份故障游戏" }).click();
    await page.getByRole("heading", { name: "自动备份故障游戏", exact: true }).waitFor();
    await page.waitForTimeout(1000);
    assert((await page.locator(".snap").count()) === 1, "failed game should retain only its baseline snapshot");
    assert(!fs.existsSync(brokenGame), "auto check recreated the missing save directory");
    assert(!fs.existsSync(tokenPath), "background auto backup created a Baidu token or opened auth flow");
    result.observations.push("missing game left no visible snapshot and its path was not recreated");
    result.observations.push("unconnected profile remained without OAuth token during background backup");
    await page.screenshot({ path: path.join(runRoot, "11-failed-game-isolated.png") });

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase5-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase5-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
