const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "U-04-startup-check", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "自动备份测试游戏", exact: true }).waitFor();
    await page.locator(".snap").nth(2).waitFor({ timeout: 15000 });
    assert((await page.locator(".snap").count()) === 3, "startup check should create exactly one new snapshot");
    assert((await page.locator(".snap").filter({ hasText: "自动快照" }).count()) === 2, "startup auto snapshot reason missing");
    assert((await page.locator(".snap").filter({ hasText: "3 个文件" }).count()) >= 1, "startup snapshot should contain three files");
    result.observations.push("modified save was captured immediately after application startup");
    await page.screenshot({ path: path.join(runRoot, "08-startup-auto-snapshot.png") });
    await page.waitForTimeout(3500);
    assert((await page.locator(".snap").count()) === 3, "startup check duplicated without another change");
    result.observations.push("no duplicate during short unchanged follow-up");
    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase3-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase3-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
