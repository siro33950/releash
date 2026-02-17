import type { Page } from "@playwright/test";

/** アプリの初期化完了を待機 */
export async function waitForApp(page: Page) {
	await page.goto("/");
	await page.waitForLoadState("load");
}

/**
 * invoke の呼び出し履歴を記録する仕組みを注入する。
 * setupTauriMock の後、page.goto() の前に呼ぶこと。
 */
export async function trackInvocations(page: Page) {
	await page.addInitScript(() => {
		// @ts-expect-error - __TAURI_INTERNALS__ は setupTauriMock で注入済み
		const original = window.__TAURI_INTERNALS__?.invoke;
		if (!original) return;

		const history: Array<{ cmd: string; args: unknown }> = [];
		// @ts-expect-error - テスト用グローバル
		window.__INVOKE_HISTORY__ = history;

		// @ts-expect-error - invoke をラップ
		window.__TAURI_INTERNALS__.invoke = (
			cmd: string,
			args: Record<string, unknown>,
		) => {
			history.push({ cmd, args });
			return original(cmd, args);
		};
	});
}

/**
 * 記録された invoke 呼び出し履歴を取得する。
 */
export async function getInvokeHistory(
	page: Page,
): Promise<Array<{ cmd: string; args: unknown }>> {
	return page.evaluate(
		// @ts-expect-error - テスト用グローバル
		() => window.__INVOKE_HISTORY__ ?? [],
	);
}

/**
 * 指定コマンドが invoke された回数を返す。
 */
export async function getInvokeCount(
	page: Page,
	cmd: string,
): Promise<number> {
	const history = await getInvokeHistory(page);
	return history.filter((h) => h.cmd === cmd).length;
}
