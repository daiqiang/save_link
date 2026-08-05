const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-auto-backup-20260805";
const goodGame = path.join(runRoot, "saves", "good-game");

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "U-09-G-01-G-02-lock-deduplicate-restore", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9229");
    const page = browser.contexts()[0].pages()[0];
    await page.locator(".game-item").filter({ hasText: "自动备份测试游戏" }).click();
    await page.getByRole("heading", { name: "自动备份测试游戏", exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 5, "good game should start phase with five snapshots");

    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.getByText("存档未变化，未创建新快照", { exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 5, "manual no-change action created a duplicate");
    result.observations.push("manual no-change action created no duplicate");

    const oldest = page.locator(".snap").last();
    await oldest.getByTitle("锁定").click();
    await oldest.locator(".badge.lock").waitFor();
    assert((await oldest.getAttribute("class")).includes("is-locked"), "locked snapshot visual class missing");
    await oldest.getByTitle("更多").click();
    const disabledDelete = page.locator(".ctx-menu").getByRole("button", { name: "锁定快照不能删除", exact: true });
    assert(await disabledDelete.isDisabled(), "locked snapshot delete action should be disabled");
    result.observations.push("locked visual state and delete protection visible");
    await page.screenshot({ path: path.join(runRoot, "12-locked-snapshot.png") });
    await page.getByRole("heading", { name: "自动备份测试游戏", exact: true }).click();

    await oldest.getByRole("button", { name: "恢复", exact: true }).click();
    await page.getByRole("heading", { name: "恢复到这个存档版本？", exact: true }).waitFor();
    await page.getByRole("button", { name: "恢复这个版本", exact: true }).click();
    await page.getByRole("heading", { name: "恢复完成", exact: true }).waitFor({ timeout: 15000 });
    await page.screenshot({ path: path.join(runRoot, "13-restored-oldest.png") });
    await page.getByRole("button", { name: "回到时间线", exact: true }).click();
    assert((await page.locator(".snap").count()) === 5, "restore should not create a backup snapshot");
    assert(!fs.existsSync(path.join(goodGame, "added-v2.sav")), "restore behaved as merge and left a newer file");
    const restored = fs.readFileSync(path.join(goodGame, "slot1.sav"), "utf8");
    assert(restored.includes("good-v1"), "restored slot1 content does not match oldest snapshot");
    assert(fs.readdirSync(goodGame).sort().join(",") === "config.ini,slot1.sav", "restored directory has unexpected files");
    result.observations.push("oldest snapshot restored by replacement with no automatic pre-restore snapshot");
    result.observations.push("unconnected local snapshot remained fully restorable");

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-phase6-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-phase6-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
