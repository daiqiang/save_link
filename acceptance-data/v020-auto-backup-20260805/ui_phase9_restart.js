const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "R-03-G-03-U-09-restart-persistence", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.locator(".game-item").filter({ hasText: "自动备份测试游戏-已编辑" }).click();
    await page.getByRole("heading", { name: "自动备份测试游戏-已编辑", exact: true }).waitFor();
    await page.locator(".snap").nth(30).waitFor({ timeout: 15000 });
    assert((await page.locator(".snap").count()) === 31, "restart changed retained snapshot count");
    assert((await page.locator(".snap:not(.is-locked)").count()) === 30, "restart changed unlocked snapshot count");
    assert((await page.locator(".snap.is-locked").count()) === 1, "locked snapshot did not survive restart");
    result.observations.push("edited name, 30+1 retention and lock state survived restart");

    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    assert((await autoSwitch.getAttribute("aria-checked")) === "true", "enabled auto-backup setting did not survive restart");
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await page.waitForTimeout(3500);
    assert((await page.locator(".snap").count()) === 31, "unchanged startup check created or removed an extra snapshot");
    await page.screenshot({ path: path.join(runRoot, "17-restart-retention-stable.png"), fullPage: true });
    result.observations.push("unchanged startup check left retention set stable");

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase9-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase9-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
