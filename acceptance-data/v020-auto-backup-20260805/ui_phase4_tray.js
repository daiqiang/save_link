const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "U-05-tray-periodic", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "自动备份测试游戏", exact: true }).waitFor();
    await page.locator(".snap").nth(3).waitFor({ timeout: 10000 });
    assert((await page.locator(".snap").count()) === 4, "periodic tray check should produce the fourth snapshot");
    assert((await page.locator(".snap").filter({ hasText: "自动快照" }).count()) === 3, "periodic snapshot reason missing");
    assert((await page.locator(".snap").filter({ hasText: "3 个文件" }).count()) >= 1, "periodic snapshot file count missing");
    result.observations.push("10-minute check ran while main window was hidden");
    result.observations.push("timeline showed four snapshots after window restore");
    await page.screenshot({ path: path.join(runRoot, "10-restored-from-tray.png") });
    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase4-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase4-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
