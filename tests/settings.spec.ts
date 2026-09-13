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
		list_worktrees: [
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
		get_current_branch: "feat/test",
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
