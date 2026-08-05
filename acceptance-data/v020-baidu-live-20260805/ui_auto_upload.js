const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-baidu-live-20260805";
const saveA = path.join(runRoot, "save-a");

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "C-01-real-auto-upload", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9230");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "SaveLink百度实测-20260805", exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 1, "auto upload phase should start with one snapshot");
    await page.locator(".snap").first().getByText("已上云", { exact: true }).waitFor();

    fs.writeFileSync(path.join(saveA, "cloud-test.sav"), "savelink-live-baidu-v2-auto\n", "utf8");
    fs.writeFileSync(path.join(saveA, "auto-extra.sav"), "auto-upload-extra\n", "utf8");
    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    assert((await autoSwitch.getAttribute("aria-checked")) === "false", "auto backup should be disabled before trigger");
    await autoSwitch.click();
    await page.locator('[role="switch"][aria-checked="true"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();

    await page.locator(".snap").nth(1).waitFor({ timeout: 30000 });
    const newest = page.locator(".snap").first();
    await newest.getByText("自动快照", { exact: false }).waitFor();
    await newest.getByText("已上云", { exact: true }).waitFor({ timeout: 120000 });
    assert((await page.locator(".snap").count()) === 2, "auto trigger created an unexpected number of snapshots");
    result.observations.push("content change created one auto snapshot and uploaded it without opening OAuth");
    await page.screenshot({ path: path.join(runRoot, "05-auto-snapshot-uploaded.png") });

    await page.getByTitle("设置").click();
    const switchOff = page.getByRole("switch", { name: "自动备份" });
    await switchOff.click();
    await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    await page.getByRole("button", { name: "完成", exact: true }).click();

    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-auto-upload-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-auto-upload-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
