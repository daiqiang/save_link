const crypto = require("crypto");
const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-timestamp-fix-20260805";
const saveA = path.join(runRoot, "save-a");
const restoreB = path.join(runRoot, "restore-b");
const gameName = "SaveLink时间修复实测-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

function sha256(file) {
  return crypto.createHash("sha256").update(fs.readFileSync(file)).digest("hex");
}

(async () => {
  const result = { phase: "real-baidu-profile-b-mixed-retention", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9242");
    const page = browser.contexts()[0].pages()[0];
    await page.getByText("设备 B 隔离测试", { exact: true }).waitFor();

    await page.getByTitle("设置").click();
    const initialSwitch = page.getByRole("switch", { name: "自动备份" });
    if ((await initialSwitch.getAttribute("aria-checked")) === "true") {
      await initialSwitch.click();
      await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    }
    await page.getByRole("button", { name: "完成", exact: true }).click();

    await page.getByRole("button", { name: "云端存档", exact: true }).click();
    const cloudModal = page.locator(".cloud-modal");
    const testGame = cloudModal.locator(".cloud-game").filter({ hasText: gameName });
    await testGame.waitFor({ timeout: 60000 });
    const cloudRow = testGame.locator(".cloud-snapshot-row").first();
    const cloudText = await cloudRow.textContent();
    assert(/^.*2026-08-0[56] \d{2}:\d{2}.*$/.test(cloudText ?? ""), `cloud timestamp was not localized: ${cloudText}`);
    assert(!/T\d{2}:\d{2}|\+08:00|Z/.test(cloudText ?? ""), `raw cloud timestamp leaked: ${cloudText}`);
    await cloudRow.getByRole("button", { name: "下载", exact: true }).click();
    await page.getByText("云端快照已下载到本机仓库", { exact: true }).waitFor({ timeout: 120000 });
    await cloudRow.getByText("已在本机", { exact: true }).waitFor();
    result.observations.push("profile B discovered and downloaded the real cloud snapshot with localized time");
    await page.screenshot({ path: path.join(runRoot, "04-profile-b-downloaded-local-time.png"), fullPage: true });
    await cloudModal.getByTitle("关闭").click();

    await page.getByRole("heading", { name: gameName, exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 1, "downloaded timeline snapshot missing");
    const downloadedTime = await page.locator(".snap-time").textContent();
    assert(/^2026-08-0[56] \d{2}:\d{2}$/.test(downloadedTime ?? ""), `timeline timestamp was not localized: ${downloadedTime}`);
    assert(!/[TZ+]/.test(downloadedTime ?? ""), "raw RFC 3339 leaked into downloaded timeline");

    await page.getByRole("button", { name: "绑定存档目录", exact: true }).click();
    const bindModal = page.locator(".modal").filter({ has: page.getByRole("heading", { name: "绑定存档目录", exact: true }) });
    await bindModal.locator("input").fill(restoreB);
    await bindModal.getByRole("button", { name: "测试读取", exact: true }).click();
    await bindModal.getByText("目录可读取，当前为空", { exact: true }).waitFor();
    await bindModal.getByRole("button", { name: "确认绑定", exact: true }).click();
    await page.getByText("本机存档目录已绑定", { exact: true }).waitFor();

    await page.locator(".snap").first().getByRole("button", { name: "恢复", exact: true }).click();
    await page.getByRole("button", { name: "恢复这个版本", exact: true }).click();
    await page.getByRole("heading", { name: "恢复完成", exact: true }).waitFor({ timeout: 30000 });
    await page.getByRole("button", { name: "回到时间线", exact: true }).click();
    const names = fs.readdirSync(restoreB).sort();
    assert(names.join(",") === "config.ini,slot-main.sav", `restored files mismatch: ${names}`);
    for (const name of names) {
      assert(sha256(path.join(restoreB, name)) === sha256(path.join(saveA, name)), `restored hash mismatch: ${name}`);
    }

    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.getByText("存档未变化，未创建新快照", { exact: true }).last().waitFor();
    assert((await page.locator(".snap").count()) === 1, "downloaded snapshot did not deduplicate against restored content");

    fs.writeFileSync(path.join(restoreB, "slot-main.sav"), "timestamp-fix-device-b-local-01\n", "utf8");
    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.waitForFunction(() => document.querySelectorAll(".snap").length === 2, undefined, { timeout: 20000 });
    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.getByText("存档未变化，未创建新快照", { exact: true }).last().waitFor();
    assert((await page.locator(".snap").count()) === 2, "no-change dedup created a duplicate after local snapshot");
    result.observations.push("downloaded-to-local and local-to-local no-change dedup both passed");

    for (let targetCount = 3; targetCount <= 31; targetCount += 1) {
      fs.writeFileSync(path.join(restoreB, "retention.sav"), `mixed-retention-${String(targetCount).padStart(2, "0")}\n`, "utf8");
      await page.getByRole("button", { name: "创建快照", exact: true }).click();
      await page.waitForFunction(
        (count) => document.querySelectorAll(".snap").length === count,
        targetCount,
        { timeout: 20000 },
      );
    }
    await page.screenshot({ path: path.join(runRoot, "05-mixed-retention-before-31.png"), fullPage: true });

    await page.getByTitle("设置").click();
    const enableSwitch = page.getByRole("switch", { name: "自动备份" });
    await enableSwitch.click();
    await page.locator('[role="switch"][aria-checked="true"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();
    await page.waitForFunction(() => document.querySelectorAll(".snap").length === 30, undefined, { timeout: 120000 });
    result.observations.push("31 mixed-source snapshots pruned to 30 through the real cloud-aware retention path");
    await page.screenshot({ path: path.join(runRoot, "06-mixed-retention-after-30.png"), fullPage: true });

    await page.getByTitle("设置").click();
    const disableSwitch = page.getByRole("switch", { name: "自动备份" });
    await disableSwitch.click();
    await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-real-b-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-real-b-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
