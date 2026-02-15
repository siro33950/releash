import { expect, test } from "@playwright/test";
import { buildMockConfig, rootDirEntries } from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * WorktreeView の ReviewPanel テスト。
 * ReviewPanel は EditorPanel の下部パネルとして表示される。
 * Terminal / Comments タブの切り替えをテストする。
 */
function reviewConfig(overrides: Record<string, unknown> = {}) {
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
		get_file_at_ref: "// original content",
		get_staged_content: "// original content",
		"plugin:fs|read_text_file": "// modified content",
		...overrides,
	});
}

test.describe("Review Panel", () => {
	test("Terminal と Comments タブが表示される", async ({ page }) => {
		const config = reviewConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// ファイルを開いて ReviewPanel を表示させる
		await page.getByRole("button", { name: "Explorer" }).click();
		await page.getByText("README.md").first().click();
		await expect(
			page.locator(".flexlayout__tab_button", { hasText: "README.md" }),
		).toBeVisible({ timeout: 5000 });

		// ReviewPanel の Terminal / Comments タブが表示される
		const terminalTab = page.getByRole("tab", { name: "Terminal" });
		const commentsTab = page.getByRole("tab", { name: /Comments/ });

		await expect(terminalTab).toBeVisible();
		await expect(commentsTab).toBeVisible();
	});

	test("Comments タブに切り替えるとコメント空メッセージが表示される", async ({
		page,
	}) => {
		const config = reviewConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// ファイルを開く
		await page.getByRole("button", { name: "Explorer" }).click();
		await page.getByText("README.md").first().click();
		await expect(
			page.locator(".flexlayout__tab_button", { hasText: "README.md" }),
		).toBeVisible({ timeout: 5000 });

		// Comments タブをクリック
		await page.getByRole("tab", { name: /Comments/ }).click();

		// 空メッセージ "コメントなし" が表示される
		await expect(page.getByText("コメントなし")).toBeVisible();
	});

	test("Terminal タブがデフォルトで選択されている", async ({ page }) => {
		const config = reviewConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// ファイルを開く
		await page.getByRole("button", { name: "Explorer" }).click();
		await page.getByText("README.md").first().click();
		await expect(
			page.locator(".flexlayout__tab_button", { hasText: "README.md" }),
		).toBeVisible({ timeout: 5000 });

		// Terminal タブが aria-selected="true" であることを確認
		const terminalTab = page.getByRole("tab", { name: "Terminal" });
		await expect(terminalTab).toHaveAttribute("aria-selected", "true");
	});
});
