const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";
const goodGame = path.join(runRoot, "saves", "good-game");

(async () => {
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.locator(".game-item").filter({ hasText: "自动备份测试游戏-已编辑" }).click();
    if ((await page.locator(".snap").count()) !== 31) throw new Error("disable test must start with 31 snapshots");
    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    if ((await autoSwitch.getAttribute("aria-checked")) !== "true") throw new Error("auto backup unexpectedly disabled before test");
    await autoSwitch.click();
    await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    fs.writeFileSync(path.join(goodGame, "slot1.sav"), "disabled-period-change\n", "utf8");
    await page.screenshot({ path: path.join(runRoot, "18-auto-disabled-before-wait.png") });
    fs.writeFileSync(path.join(runRoot, "ui-phase10-prepare-result.json"), JSON.stringify({ passed: true, count: 31 }, null, 2));
    console.log(JSON.stringify({ passed: true, count: 31 }));
    process.exit(0);
  } catch (error) {
    fs.writeFileSync(path.join(runRoot, "ui-phase10-prepare-result.json"), JSON.stringify({ passed: false, error: String(error.stack || error) }, null, 2));
    console.error(error.stack || error);
    process.exit(1);
  }
})();
