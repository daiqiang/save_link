const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "U-01-default-and-disable", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const contexts = browser.contexts();
    assert(contexts.length === 1, `expected one browser context, got ${contexts.length}`);
    const pages = contexts[0].pages();
    assert(pages.length >= 1, "no SaveLink page found");
    const page = pages[0];
    await page.waitForLoadState("domcontentloaded");
    await page.getByText("设备 B 隔离测试", { exact: true }).waitFor();
    result.observations.push("isolated profile label visible");

    await page.getByTitle("设置").click();
    const dialog = page.getByRole("heading", { name: "设置", exact: true }).locator("..", { has: page.getByRole("switch") });
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    await autoSwitch.waitFor();
    const initial = await autoSwitch.getAttribute("aria-checked");
    assert(initial === "true", `auto backup should default to true, got ${initial}`);
    await page.getByText("每 10 分钟检查一次", { exact: true }).waitFor();
    result.observations.push("default enabled and 10-minute interval visible");
    await page.screenshot({ path: path.join(runRoot, "02-settings-default-on.png") });

    await autoSwitch.click();
    await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    await page.getByText("自动备份已关闭", { exact: true }).waitFor();
    result.observations.push("toggle disabled and success toast visible");
    await page.screenshot({ path: path.join(runRoot, "03-settings-off.png") });

    await page.getByRole("button", { name: "完成", exact: true }).click();
    await page.getByRole("heading", { name: "设置", exact: true }).waitFor({ state: "detached" });
    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase1-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase1-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exitCode = 1;
  }
})();
