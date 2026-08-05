const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-baidu-live-20260805";
const saveA = path.join(runRoot, "save-a");
const gameName = "SaveLink百度实测-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "C-01-oauth-manual-upload", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9230");
    const page = browser.contexts()[0].pages()[0];
    await page.getByText("设备 B 隔离测试", { exact: true }).waitFor();

    await page.getByTitle("设置").click();
    const autoSwitch = page.getByRole("switch", { name: "自动备份" });
    if ((await autoSwitch.getAttribute("aria-checked")) === "true") {
      await autoSwitch.click();
      await page.locator('[role="switch"][aria-checked="false"]').waitFor();
    }
    await page.getByRole("button", { name: "完成", exact: true }).click();

    await page.getByRole("button", { name: "添加游戏", exact: true }).last().click();
    const modal = page.locator(".modal");
    await modal.locator("input").nth(0).fill(gameName);
    await modal.locator("input").nth(1).fill(saveA);
    await modal.getByRole("button", { name: "测试读取", exact: true }).click();
    await modal.getByText(/已检测到：2 个文件/).waitFor();
    await modal.getByRole("button", { name: "保存并创建", exact: true }).click();
    await page.getByRole("heading", { name: gameName, exact: true }).waitFor();
    await page.getByRole("button", { name: "创建快照", exact: true }).click();
    await page.getByText("快照已创建", { exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 1, "manual cloud baseline snapshot missing");
    await page.screenshot({ path: path.join(runRoot, "01-before-oauth-upload.png") });

    console.log("AUTH_PAGE_OPENING");
    const upload = page.locator(".snap").first().locator(".cloud-upload");
    await upload.click();
    await page.getByText("已上云", { exact: true }).waitFor({ timeout: 300000 });
    await page.getByText(/快照已保存到百度网盘|这条快照已经保存在百度网盘/).waitFor();
    result.observations.push("OAuth callback completed and the manual snapshot reached uploaded state");
    await page.screenshot({ path: path.join(runRoot, "02-manual-snapshot-uploaded.png") });
    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-auth-upload-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-auth-upload-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
