const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-timestamp-fix-20260805";
const restoreB = path.join(runRoot, "restore-b");
const gameName = "SaveLink时间修复实测-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "real-baidu-profile-b-retention-continuation", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9242");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: gameName, exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 2, "continuation must start with downloaded plus local snapshot");
    const displayedTimes = await page.locator(".snap-time").allTextContents();
    assert(displayedTimes.every((value) => /^2026-08-0[56] \d{2}:\d{2}$/.test(value)), `unexpected timeline times: ${displayedTimes}`);
    assert(displayedTimes.every((value) => !/[TZ+]/.test(value)), "raw timestamp leaked into mixed timeline");

    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.getByText("存档未变化，未创建新快照", { exact: true }).last().waitFor();
    assert((await page.locator(".snap").count()) === 2, "continuation no-change check created a duplicate");

    for (let targetCount = 3; targetCount <= 31; targetCount += 1) {
      fs.writeFileSync(path.join(restoreB, "retention.sav"), `mixed-retention-${String(targetCount).padStart(2, "0")}\n`, "utf8");
      await page.getByRole("button", { name: "创建快照", exact: true }).click();
      await page.waitForFunction(
        (count) => document.querySelectorAll(".snap").length === count,
        targetCount,
        { timeout: 20000 },
      );
    }
    result.observations.push("constructed 31 mixed-source snapshots without duplicate creation");
    await page.screenshot({ path: path.join(runRoot, "05-mixed-retention-before-31.png"), fullPage: true });

    await page.getByTitle("设置").click();
    const enableSwitch = page.getByRole("switch", { name: "自动备份" });
    assert((await enableSwitch.getAttribute("aria-checked")) === "false", "auto backup must be disabled before prune");
    await enableSwitch.click();
    await page.locator('[role="switch"][aria-checked="true"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await page.waitForFunction(() => document.querySelectorAll(".snap").length === 30, undefined, { timeout: 120000 });
    result.observations.push("real cloud-aware retention pruned 31 mixed-source snapshots to 30");
    await page.screenshot({ path: path.join(runRoot, "06-mixed-retention-after-30.png"), fullPage: true });

    await page.getByTitle("设置").click();
    const disableSwitch = page.getByRole("switch", { name: "自动备份" });
    await disableSwitch.click();
    await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    result.observations.push("auto backup disabled after destructive verification");

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-real-b-retention-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-real-b-retention-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
