const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";
const goodGame = path.join(runRoot, "saves", "good-game");

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "R-02-retention-30-unlocked", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.locator(".game-item").filter({ hasText: "自动备份测试游戏-已编辑" }).click();
    await page.getByRole("heading", { name: "自动备份测试游戏-已编辑", exact: true }).waitFor();

    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    assert((await autoSwitch.getAttribute("aria-checked")) === "false", "retention construction requires auto backup disabled");
    await page.getByRole("button", { name: "完成", exact: true }).click();

    const initialCount = await page.locator(".snap").count();
    assert(initialCount >= 1 && initialCount < 32, `unexpected initial snapshot count ${initialCount}`);
    assert((await page.locator(".snap.is-locked").count()) === 1, "expected exactly one locked snapshot before retention construction");

    for (let targetCount = initialCount + 1; targetCount <= 32; targetCount += 1) {
      fs.writeFileSync(path.join(goodGame, "slot1.sav"), `retention-${String(targetCount).padStart(2, "0")}\n`, "utf8");
      await page.getByRole("button", { name: "创建快照", exact: true }).click();
      await page.waitForFunction(
        (count) => document.querySelectorAll(".snap").length === count,
        targetCount,
        { timeout: 15000 },
      );
    }
    assert((await page.locator(".snap").count()) === 32, "failed to construct 32 snapshots");
    assert((await page.locator(".snap.is-locked").count()) === 1, "locked snapshot disappeared during construction");
    result.observations.push(`constructed 32 snapshots from initial count ${initialCount}`);
    await page.screenshot({ path: path.join(runRoot, "15-before-retention-32.png"), fullPage: true });

    await page.getByTitle("设置").click();
    const enableSwitch = page.getByRole("switch", { name: "自动备份" });
    await enableSwitch.click();
    await page.locator('[role="switch"][aria-checked="true"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await page.waitForFunction(
      () => document.querySelectorAll(".snap").length === 31,
      undefined,
      { timeout: 20000 },
    );
    assert((await page.locator(".snap:not(.is-locked)").count()) === 30, "unlocked snapshot count should be 30 after prune");
    assert((await page.locator(".snap.is-locked").count()) === 1, "locked snapshot should survive prune");
    assert((await page.locator(".snap.is-locked").filter({ hasText: "已锁定" }).count()) === 1, "locked badge missing after prune");
    result.observations.push("retention kept 30 unlocked plus one locked snapshot");
    await page.screenshot({ path: path.join(runRoot, "16-after-retention-31.png"), fullPage: true });

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase8-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase8-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
