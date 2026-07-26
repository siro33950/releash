import type { Page } from "@playwright/test";

/** アプリの初期化完了を待機 */
export async function waitForApp(page: Page) {
	await page.goto("/");
	await page.waitForLoadState("domcontentloaded");
	// アプリのUIが描画されるのを待機
	await page.locator("#root").waitFor({ state: "attached" });
	await page.waitForTimeout(500);
}

/**
 * Session 選択直後は権威 read の決着に伴う Workspace tree の再取得が続く。
 * mock 応答を差し替えるテストは、その再取得が止まってから差し替えないと
 * 差し替え後のスナップショットを意図しない時点で読み込んでしまう。
 */
export async function waitForWorkspaceTreeQuiescence(page: Page) {
	let previous = -1;
	for (let attempt = 0; attempt < 40; attempt += 1) {
		const current = await page.evaluate(
			() =>
				(window.__TAURI_INTERNALS__?.invocations ?? []).filter(
					(entry) => entry.cmd === "list_workspace_worktree_nodes",
				).length,
		);
		if (current === previous) return;
		previous = current;
		await page.waitForTimeout(250);
	}
}
