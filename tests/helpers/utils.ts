import type { Page } from "@playwright/test";

/** アプリの初期化完了を待機 */
export async function waitForApp(page: Page) {
	await page.goto("/");
	await page.waitForLoadState("domcontentloaded");
	// アプリのUIが描画されるのを待機
	await page.locator("#root").waitFor({ state: "attached" });
	await page.waitForTimeout(500);
}
