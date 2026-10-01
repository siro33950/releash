import { type Locator, type Page, expect, test } from "@playwright/test";
import { buildMockConfig } from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * WorktreeView の Settings パネルテスト。
 * worktree が1つだけ→自動でWorktreeView→ActivityBarでSettingsに切替。
 */
function settingsConfig(overrides: Record<string, unknown> = {}) {
	return buildMockConfig({
		"worktrees": [
			{
				name: "repo",
				path: "/test/repo",
				branch: "feat/test",
				is_main: true,
				is_locked: false,
				dirty_count: 0,
				base_branch: null,
			},
		],
		"current-branch": "feat/test",
		get_git_status: [],
		...overrides,
	});
}

/** Radix UI Select のオプションを選択するヘルパー */
async function selectRadixOption(
	page: Page,
	trigger: Locator,
	optionText: string,
) {
	await trigger.click();
	await page.getByRole("option", { name: optionText }).click();
}

test.describe("Settings", () => {
	for (const method of ["pointer", "keyboard"] as const) {
		test(`Failed の覆いを ${method} で操作しても設定の入力を保持する`, async ({ page }) => {
			await setupTauriMock(page, settingsConfig());
			await waitForApp(page);
			await page.getByRole("button", { name: "Settings" }).click();
			await page.getByRole("button", { name: "Agent", exact: true }).click();
			const input = page.locator("#terminal-startup-cmd");
			await input.fill("unsaved command");
			const publish = async (phase: string) => {
				await page.evaluate((phase) => {
					const subscription = window.__TAURI_INTERNALS__.ipcInvocations
						.filter(({ cmd }) => cmd === "subscribe_daemon_status").at(-1);
					if (!subscription) throw new Error("Missing daemon subscription");
					(subscription.args.channel as { onmessage: (status: unknown) => void }).onmessage({
						phase, retryAvailable: true, reason: "Daemon failed",
					});
				}, phase);
			};
			await publish("failed");
			const retry = page.getByRole("button", { name: "Retry", exact: true });
			const quit = page.getByRole("button", { name: "Quit", exact: true });
			if (method === "pointer") {
				await retry.click();
				await quit.click();
			} else {
				await expect(retry).toBeFocused();
				await page.keyboard.press("Enter");
				await page.keyboard.press("Tab");
				await expect(quit).toBeFocused();
				await page.keyboard.press("Enter");
			}
			await expect.poll(() => page.evaluate(() => window.__TAURI_INTERNALS__.ipcInvocations
				.filter(({ cmd }) => cmd === "retry_daemon" || cmd === "quit_desktop")
				.map(({ cmd }) => cmd))).toEqual(["retry_daemon", "quit_desktop"]);
			await publish("ready");
			await expect(retry).toHaveCount(0);
			await expect(input).toBeVisible();
			await expect(input).toHaveValue("unsaved command");
			await input.fill("continued command");
			await expect(input).toHaveValue("continued command");
		});
	}

	test("ActivityBar の Settings クリックで設定モーダルが表示される", async ({
		page,
	}) => {
		const config = settingsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// ActivityBar の Settings ボタン（aria-label="Settings"）
		const settingsBtn = page.getByRole("button", { name: "Settings" });
		await settingsBtn.click();

		// Settings モーダルが開き、デフォルトの Appearance セクションが表示される
		await expect(page.locator("#theme-select")).toBeVisible();

		// Editor セクションに切り替えると Default Base セレクトが表示される
		await expect(async () => {
			await page
				.getByRole("button", { name: "Editor" })
				.click({ timeout: 1_000 });
		}).toPass();
		await expect(page.locator("#diff-base-select")).toBeVisible();
	});

	test("テーマ切替: Dark/Light が選択可能", async ({ page }) => {
		const config = settingsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Settings パネルを開く
		const settingsBtn = page.getByRole("button", { name: "Settings" });
		await settingsBtn.click();

		// Theme セレクトで Light を選択
		const themeSelect = page.locator("#theme-select");
		await selectRadixOption(page, themeSelect, "Light");

		// 選択値が Light になっていることを確認
		await expect(themeSelect).toHaveText("Light");

		// Dark に戻す
		await selectRadixOption(page, themeSelect, "Dark");
		await expect(themeSelect).toHaveText("Dark");
	});

	test("Diff Mode のオプションが選択可能", async ({ page }) => {
		const config = settingsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		const settingsBtn = page.getByRole("button", { name: "Settings" });
		await settingsBtn.click();

		// Editor セクションに切り替え
		await page.getByRole("button", { name: "Editor" }).click();

		const diffModeSelect = page.locator("#diff-mode-select");
		await expect(diffModeSelect).toBeVisible();

		// Gutter / Inline / Split のオプションが存在する
		await selectRadixOption(page, diffModeSelect, "Gutter");
		await expect(diffModeSelect).toHaveText("Gutter");

		await selectRadixOption(page, diffModeSelect, "Split");
		await expect(diffModeSelect).toHaveText("Split");
	});

	test("Save ボタンは変更がない場合 disabled", async ({ page }) => {
		const config = settingsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		const settingsBtn = page.getByRole("button", { name: "Settings" });
		await settingsBtn.click();

		// 初期状態では Save は disabled
		const saveBtn = page.getByRole("button", { name: "Save" });
		await expect(saveBtn).toBeDisabled();

		// テーマを変更すると enabled になる
		const themeSelect = page.locator("#theme-select");
		await selectRadixOption(page, themeSelect, "Light");
		await expect(saveBtn).toBeEnabled();
	});

	test("クラッシュレポート設定トグルが表示される", async ({ page }) => {
		const config = settingsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		const settingsBtn = page.getByRole("button", { name: "Settings" });
		await settingsBtn.click();

		// Privacy & Updates セクションに切り替え
		await page.getByText("Privacy & Updates").click();

		await expect(page.getByText("Send crash reports")).toBeVisible();
	});
});
