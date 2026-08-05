const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";
const removeGame = path.join(runRoot, "saves", "remove-game");
const removeFile = path.join(removeGame, "remove.sav");

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

async function setAutoBackup(page, enabled) {
  await page.getByTitle("设置").click();
  const autoSwitch = page.getByRole("switch", { name: "自动备份" });
  const current = (await autoSwitch.getAttribute("aria-checked")) === "true";
  if (current !== enabled) {
    await autoSwitch.click();
    await page.locator(`[role="switch"][aria-checked="${enabled}"]`).waitFor();
  }
  await page.getByRole("button", { name: "完成", exact: true }).click();
}

(async () => {
  const result = { phase: "G-03-G-04-edit-remove", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.locator(".game-item").filter({ hasText: "自动备份测试游戏" }).click();
    await setAutoBackup(page, false);

    await page.getByRole("button", { name: "编辑游戏", exact: true }).click();
    let modal = page.locator(".modal").filter({ has: page.getByRole("heading", { name: "编辑游戏", exact: true }) });
    await modal.locator("input").nth(0).fill("自动备份测试游戏-已编辑");
    await modal.getByRole("button", { name: "测试读取", exact: true }).click();
    await modal.getByText(/已检测到：2 个文件/).waitFor();
    await modal.getByRole("button", { name: "保存修改", exact: true }).click();
    await page.getByRole("heading", { name: "自动备份测试游戏-已编辑", exact: true }).waitFor();
    result.observations.push("edited game name and readable path saved");

    const beforeHash = sha256(removeFile);
    await page.getByTitle("添加游戏").click();
    modal = page.locator(".modal").filter({ has: page.getByRole("heading", { name: "添加游戏", exact: true }) });
    await modal.locator("input").nth(0).fill("待移除测试游戏");
    await modal.locator("input").nth(1).fill(removeGame);
    await modal.getByRole("button", { name: "测试读取", exact: true }).click();
    await modal.getByText(/已检测到：1 个文件/).waitFor();
    await modal.getByRole("button", { name: "保存并创建", exact: true }).click();
    await page.getByRole("heading", { name: "待移除测试游戏", exact: true }).waitFor();
    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.getByText("快照已创建", { exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 1, "remove-game baseline snapshot missing");

    await page.getByRole("button", { name: "编辑游戏", exact: true }).click();
    modal = page.locator(".modal").filter({ has: page.getByRole("heading", { name: "编辑游戏", exact: true }) });
    await modal.getByRole("button", { name: "移除游戏", exact: true }).click();
    const confirm = page.locator(".modal").filter({ has: page.getByRole("heading", { name: "移除游戏？", exact: true }) });
    await confirm.getByRole("button", { name: "确认移除", exact: true }).click();
    await page.getByText("游戏已移除", { exact: true }).waitFor();
    await page.waitForTimeout(500);
    assert((await page.locator(".game-item").filter({ hasText: "待移除测试游戏" }).count()) === 0, "removed game remained in sidebar");
    assert(fs.existsSync(removeFile), "removing game deleted the real save file");
    assert(sha256(removeFile) === beforeHash, "removing game modified the real save file");
    result.observations.push("removed game metadata and repository copy while preserving real save hash");
    await page.screenshot({ path: path.join(runRoot, "14-game-edited-and-remove-safe.png") });

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase7-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase7-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
