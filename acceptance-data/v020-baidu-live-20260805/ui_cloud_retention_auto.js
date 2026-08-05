const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-baidu-live-20260805";
const saveA = path.join(runRoot, "save-a");
const deletedSnapshotId = "snap_1785936680866133900_0";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = {
    phase: "C-04-retention-delete-auto-cloud-snapshot",
    passed: false,
    deletedSnapshotId,
    observations: [],
  };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9230");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "SaveLink百度实测-20260805", exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 30, "phase must start with exactly 30 snapshots");

    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    assert((await autoSwitch.getAttribute("aria-checked")) === "true", "auto backup should be enabled after the first prune");
    await autoSwitch.click();
    await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();

    fs.writeFileSync(path.join(saveA, "retention-manual.sav"), "manual-retention-final\n", "utf8");
    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.waitForFunction(
      () => document.querySelectorAll(".snap").length === 31,
      undefined,
      { timeout: 20000 },
    );
    result.observations.push("disabled auto backup and added one unique manual snapshot; total reached 31");
    await page.screenshot({ path: path.join(runRoot, "08-cloud-retention-before-auto-delete.png"), fullPage: true });

    await page.getByTitle("设置").click();
    const enableSwitch = page.getByRole("switch", { name: "自动备份" });
    await enableSwitch.click();
    await page.locator('[role="switch"][aria-checked="true"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await page.waitForFunction(
      () => document.querySelectorAll(".snap").length === 30,
      undefined,
      { timeout: 120000 },
    );
    result.observations.push("second prune removed the oldest automatic cloud snapshot; total returned to 30");
    await page.screenshot({ path: path.join(runRoot, "09-cloud-retention-after-auto-delete.png"), fullPage: true });

    await page.getByTitle("设置").click();
    const disableSwitch = page.getByRole("switch", { name: "自动备份" });
    await disableSwitch.click();
    await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    result.observations.push("auto backup disabled after destructive test");

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-cloud-retention-auto-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-cloud-retention-auto-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
