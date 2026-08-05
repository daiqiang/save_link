const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "U-06-disabled-period", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.locator(".game-item").filter({ hasText: "自动备份测试游戏-已编辑" }).click();
    await page.getByRole("heading", { name: "自动备份测试游戏-已编辑", exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 31, "disabled 10-minute period created an automatic snapshot");
    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    assert((await autoSwitch.getAttribute("aria-checked")) === "false", "disabled setting changed during wait");
    await page.getByRole("button", { name: "完成", exact: true }).click();
    result.observations.push("full disabled period created no automatic snapshot");

    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.waitForFunction(() => document.querySelectorAll(".snap").length === 32, undefined, { timeout: 15000 });
    assert((await page.locator(".snap").first().filter({ hasText: "手动创建" }).count()) === 1, "manual snapshot was not created while auto backup disabled");
    result.observations.push("manual snapshot remained available while automatic backup was disabled");
    await page.screenshot({ path: path.join(runRoot, "19-disabled-period-manual-still-works.png"), fullPage: true });
    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase10-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase10-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
