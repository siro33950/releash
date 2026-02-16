import { expect, test } from "@playwright/test";
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

test.describe("Settings", () => {
	test("ActivityBar の Settings クリックで設定パネルが表示される", async ({
		page,
	}) => {
		const config = settingsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// ActivityBar の Settings ボタン（aria-label="Settings"）
		const settingsBtn = page.getByRole("button", { name: "Settings" });
		await settingsBtn.click();

		// Settings パネルのヘッダー（"SETTINGS" テキストがパネル上部に表示される）
		await expect(page.locator("#theme-select")).toBeVisible();
		// Default Base セレクトも表示される
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
		await themeSelect.selectOption("light");

		// 選択値が Light になっていることを確認
		await expect(themeSelect).toHaveValue("light");

		// Dark に戻す
		await themeSelect.selectOption("dark");
		await expect(themeSelect).toHaveValue("dark");
	});

	test("Diff Mode のオプションが選択可能", async ({ page }) => {
		const config = settingsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		const settingsBtn = page.getByRole("button", { name: "Settings" });
		await settingsBtn.click();

		const diffModeSelect = page.locator("#diff-mode-select");
		await expect(diffModeSelect).toBeVisible();

		// Gutter / Inline / Split のオプションが存在する
		await diffModeSelect.selectOption("gutter");
		await expect(diffModeSelect).toHaveValue("gutter");

		await diffModeSelect.selectOption("split");
		await expect(diffModeSelect).toHaveValue("split");
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
		await themeSelect.selectOption("light");
		await expect(saveBtn).toBeEnabled();
	});

	test("クラッシュレポート設定トグルが表示される", async ({ page }) => {
		const config = settingsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		const settingsBtn = page.getByRole("button", { name: "Settings" });
		await settingsBtn.click();

		await expect(page.getByText("Send crash reports")).toBeVisible();
	});
});
