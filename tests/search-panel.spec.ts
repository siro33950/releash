import { expect, test } from "@playwright/test";
import { buildMockConfig, searchResults } from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * WorktreeView の SearchPanel テスト。
 * ActivityBar の Search ボタンで切り替え。
 */
function searchConfig(overrides: Record<string, unknown> = {}) {
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

test.describe("Search Panel", () => {
	test("Search ビューに切り替えると検索入力が表示される", async ({
		page,
	}) => {
		const config = searchConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// ActivityBar の Search ボタンをクリック
		await page.getByRole("tab", { name: "Search" }).click();

		// 検索入力欄が表示される
		await expect(
			page.getByPlaceholder("Search files..."),
		).toBeVisible();
	});

	test("検索入力後にデバウンスで結果が表示される", async ({ page }) => {
		const config = searchConfig({
			search_files: searchResults,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Search ビューに切り替え
		await page.getByRole("tab", { name: "Search" }).click();

		// 検索クエリを入力
		const searchInput = page.getByPlaceholder("Search files...");
		await searchInput.fill("useState");

		// デバウンス（300ms）後に結果が表示される
		await expect(page.getByText("3 results in 2 files")).toBeVisible({
			timeout: 5000,
		});

		// ファイルグループが表示される
		await expect(page.getByText("src/App.tsx")).toBeVisible();
		await expect(page.getByText("src/main.ts")).toBeVisible();
	});

	test("Match Case トグルが動作する", async ({ page }) => {
		const config = searchConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByRole("tab", { name: "Search" }).click();

		// Match Case ボタン（data-testid="toggle-case"）
		const caseBtn = page.locator('[data-testid="toggle-case"]');
		await expect(caseBtn).toBeVisible();

		// クリックでトグル
		await caseBtn.click();

		// bg-muted クラスが追加されていること（アクティブ状態）を確認
		await expect(caseBtn).toHaveClass(/bg-muted/);
	});

	test("Use Regular Expression トグルが動作する", async ({ page }) => {
		const config = searchConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByRole("tab", { name: "Search" }).click();

		// Regex ボタン（data-testid="toggle-regex"）
		const regexBtn = page.locator('[data-testid="toggle-regex"]');
		await expect(regexBtn).toBeVisible();

		// クリックでトグル
		await regexBtn.click();

		// bg-muted クラスが追加されていること（アクティブ状態）を確認
		await expect(regexBtn).toHaveClass(/bg-muted/);
	});

	test("Clear ボタンで検索がリセットされる", async ({ page }) => {
		const config = searchConfig({
			search_files: searchResults,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByRole("tab", { name: "Search" }).click();

		// 検索入力
		const searchInput = page.getByPlaceholder("Search files...");
		await searchInput.fill("useState");

		// 結果が表示されるまで待機
		await expect(page.getByText("3 results in 2 files")).toBeVisible({
			timeout: 5000,
		});

		// Clear ボタンをクリック
		await page.getByTitle("Clear").click();

		// 入力欄が空になる
		await expect(searchInput).toHaveValue("");

		// 結果が消える
		await expect(page.getByText("3 results in 2 files")).not.toBeVisible();
	});
});
