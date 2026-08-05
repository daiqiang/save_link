const fs = require("fs");
const path = require("path");
const { chromium } = require("C:/Users/daiqiang/.cache/codex-runtimes/codex-primary-runtime/dependencies/node/node_modules/.pnpm/playwright-core@1.61.1/node_modules/playwright-core");

const runRoot = "C:/Users/daiqiang/door/project_workspace/save_link_workspace/acceptance-data/v020-timestamp-fix-20260805";

function assert(condition, message) {
  if (!condition) throw new Error(message);
}

(async () => {
  const result = { phase: "legacy-profile-migration-ui", passed: false, observations: [] };
  try {
    const browser = await chromium.connectOverCDP("http://127.0.0.1:9240");
    const page = browser.contexts()[0].pages()[0];
    await page.getByRole("heading", { name: "SaveLink百度实测-20260805", exact: true }).waitFor();
    assert((await page.locator(".snap").count()) === 30, "snapshot count changed during migration");
    const times = await page.locator(".snap-time").allTextContents();
    assert(times.length === 30, "timeline did not render every snapshot");
    assert(times.every((value) => /^2026-08-05 21:3[56]$/.test(value)), `unexpected formatted values: ${JSON.stringify([...new Set(times)])}`);
    assert(times.every((value) => !/[TZ]/.test(value)), "raw RFC 3339 leaked into timeline");
    const latest = await page.locator(".gstat").filter({ hasText: "最近快照" }).locator(".v").textContent();
    assert(latest === "2026-08-05 21:36", `unexpected latest snapshot display: ${latest}`);
    result.observations.push("30 legacy snapshots rendered in one local display format");
    result.observations.push("latest snapshot display remained 2026-08-05 21:36");
    await page.screenshot({ path: path.join(runRoot, "01-legacy-profile-migrated.png"), fullPage: true });
    result.passed = true;
    fs.writeFileSync(path.join(runRoot, "ui-migration-result.json"), JSON.stringify(result, null, 2));
    console.log(JSON.stringify(result));
    process.exit(0);
  } catch (error) {
    result.error = String(error && error.stack ? error.stack : error);
    fs.writeFileSync(path.join(runRoot, "ui-migration-result.json"), JSON.stringify(result, null, 2));
    console.error(result.error);
    process.exit(1);
  }
})();
