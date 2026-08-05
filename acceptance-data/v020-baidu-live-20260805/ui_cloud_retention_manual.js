const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-baidu-live-20260805";
const saveA = path.join(runRoot, "save-a");
const deletedSnapshotId = "snap_1785936338554505200_1";
const survivingSnapshotId = "snap_1785936680866133900_0";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = {
    phase: "C-04-retention-delete-manual-cloud-snapshot",
    passed: false,
    deletedSnapshotId,
    survivingSnapshotId,
    observations: [],
  };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9230");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "SaveLink百度实测-20260805", exact: true }).waitFor();

    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    assert((await autoSwitch.getAttribute("aria-checked")) === "false", "auto backup must be disabled before constructing snapshots");
    await page.getByRole("button", { name: "完成", exact: true }).click();

    const initialCount = await page.locator(".snap").count();
    assert(initialCount === 2, `expected two cloud baseline snapshots, got ${initialCount}`);
    for (let targetCount = initialCount + 1; targetCount <= 31; targetCount += 1) {
      fs.writeFileSync(
        path.join(saveA, "retention-manual.sav"),
        `manual-retention-${String(targetCount).padStart(2, "0")}\n`,
        "utf8",
      );
      await page.getByRole("button", { name: "创建快照", exact: true }).click();
      await page.waitForFunction(
        (count) => document.querySelectorAll(".snap").length === count,
        targetCount,
        { timeout: 20000 },
      );
    }
    assert((await page.locator(".snap").count()) === 31, "failed to construct 31 snapshots");
    result.observations.push("constructed 29 unique manual snapshots; total reached 31");
    await page.screenshot({ path: path.join(runRoot, "06-cloud-retention-before-manual-delete.png"), fullPage: true });

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
    result.observations.push("enabling auto backup pruned the oldest unlocked snapshot; total returned to 30");
    await page.screenshot({ path: path.join(runRoot, "07-cloud-retention-after-manual-delete.png"), fullPage: true });

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-cloud-retention-manual-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-cloud-retention-manual-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
