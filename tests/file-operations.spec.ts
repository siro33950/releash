import { expect, test } from "@playwright/test";
import {
	buildMockConfig,
	fsPluginCommands,
	rootDirEntries,
} from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { trackInvocations, waitForApp } from "./helpers/utils";

/**
 * WorktreeView の Explorer ファイル操作テスト。
 * SidebarPanel のツールバーボタン（New File / New Folder / Collapse All / Refresh）
 */
function fileOpsConfig(overrides: Record<string, unknown> = {}) {
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
		"plugin:fs|read_dir": rootDirEntries,
		...fsPluginCommands,
		...overrides,
	});
}

test.describe("File Operations", () => {
	test("New File ボタンクリックでインライン入力が表示される", async ({
		page,
	}) => {
		const config = fileOpsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Explorer ビューに切り替え
		await page.getByRole("button", { name: "Explorer" }).click();

		// ファイルツリーが表示されるまで待機
		await expect(page.getByText("src").first()).toBeVisible();

		// New File ボタン（aria-label="New File"）をクリック
		await page.getByRole("button", { name: "New File" }).click();

		// InlineInput が表示される（input要素がファイルツリー内に出現）
		const inlineInput = page.locator(
			".bg-input.border-primary",
		);
		await expect(inlineInput).toBeVisible({ timeout: 3000 });
	});

	test("New Folder ボタンクリックでインライン入力が表示される", async ({
		page,
	}) => {
		const config = fileOpsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByRole("button", { name: "Explorer" }).click();
		await expect(page.getByText("src").first()).toBeVisible();

		// New Folder ボタン（aria-label="New Folder"）をクリック
		await page.getByRole("button", { name: "New Folder" }).click();

		// InlineInput が表示される
		const inlineInput = page.locator(
			".bg-input.border-primary",
		);
		await expect(inlineInput).toBeVisible({ timeout: 3000 });
	});

	test("New File でファイル名を入力して Enter で作成される", async ({
		page,
	}) => {
		const config = fileOpsConfig();
		await setupTauriMock(page, config);
		await trackInvocations(page);
		await waitForApp(page);

		await page.getByRole("button", { name: "Explorer" }).click();
		await expect(page.getByText("src").first()).toBeVisible();

		// New File ボタンをクリック
		await page.getByRole("button", { name: "New File" }).click();

		// InlineInput にファイル名を入力して Enter
		const inlineInput = page.locator(
			".bg-input.border-primary",
		);
		await expect(inlineInput).toBeVisible({ timeout: 3000 });
		await inlineInput.fill("newfile.ts");
		await inlineInput.press("Enter");

		// plugin:fs|write_text_file が呼ばれることを確認
		await page.waitForFunction(
			() =>
				// @ts-expect-error - テスト用グローバル
				(window.__INVOKE_HISTORY__ ?? []).some(
					(h: { cmd: string }) => h.cmd === "plugin:fs|write_text_file",
				),
			null,
			{ timeout: 5000 },
		);
	});

	test("Collapse All ボタンが存在し、クリック可能", async ({ page }) => {
		const config = fileOpsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByRole("button", { name: "Explorer" }).click();
		await expect(page.getByText("src").first()).toBeVisible();

		// Collapse All ボタンが表示される
		const collapseBtn = page.getByRole("button", { name: "Collapse All" });
		await expect(collapseBtn).toBeVisible();

		// クリックしてもエラーにならない
		await collapseBtn.click();
	});

	test("Refresh ボタンが存在し、クリック可能", async ({ page }) => {
		const config = fileOpsConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByRole("button", { name: "Explorer" }).click();
		await expect(page.getByText("src").first()).toBeVisible();

		// Refresh ボタンが表示される
		const refreshBtn = page.getByRole("button", { name: "Refresh" });
		await expect(refreshBtn).toBeVisible();

		// クリックしてもエラーにならない
		await refreshBtn.click();
	});
});
