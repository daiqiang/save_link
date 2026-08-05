const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";
const goodGame = path.join(runRoot, "saves", "good-game");

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "U-02-U-03-continue", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "自动备份测试游戏", exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 1, "expected one existing baseline snapshot");

    fs.writeFileSync(path.join(goodGame, "slot1.sav"), "good-v2\n", "utf8");
    fs.writeFileSync(path.join(goodGame, "added-v2.sav"), "new-file-v2\n", "utf8");

    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    assert((await autoSwitch.getAttribute("aria-checked")) === "false", "continuation expected persisted disabled setting");
    await autoSwitch.click();
    await page.locator('[role="switch"][aria-checked="true"]').waitFor();
    await page.getByText("自动备份已开启", { exact: true }).waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await page.locator(".snap").nth(1).waitFor({ timeout: 15000 });
    assert((await page.locator(".snap").count()) === 2, "immediate auto check should create exactly one snapshot");
    assert((await page.locator(".snap").filter({ hasText: "自动快照" }).count()) === 1, "auto snapshot reason label missing");
    result.observations.push("enabling triggered one immediate auto snapshot and timeline refresh");
    await page.screenshot({ path: path.join(runRoot, "06-immediate-auto-snapshot.png") });

    await page.getByTitle("设置").click();
    const switchAgain = page.getByRole("switch", { name: "自动备份" });
    await switchAgain.click();
    await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    await switchAgain.click();
    await page.locator('[role="switch"][aria-checked="true"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await page.waitForTimeout(3500);
    assert((await page.locator(".snap").count()) === 2, "unchanged content created a duplicate auto snapshot");
    result.observations.push("unchanged immediate recheck created no duplicate");
    await page.screenshot({ path: path.join(runRoot, "07-no-change-no-duplicate.png") });

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase2-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase2-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
