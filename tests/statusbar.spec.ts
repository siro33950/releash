import { expect, test } from "@playwright/test";
import { buildMockConfig, rootDirEntries } from "./helpers/fixtures";
import { setupTauriMock, emitTauriEvent } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * WorktreeView の StatusBar テスト。
 * worktree が1つ→自動でWorktreeView が開き、下部に StatusBar が表示される。
 */
function statusBarConfig(overrides: Record<string, unknown> = {}) {
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
		get_current_branch: "feat/my-branch",
		get_git_status: [],
		...overrides,
	});
}

test.describe("StatusBar", () => {
	test("ブランチ名が表示される", async ({ page }) => {
		const config = statusBarConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// StatusBar にブランチ名が表示される
		await expect(page.getByText("feat/my-branch")).toBeVisible();
	});

	test("ファイルを開くと言語情報が表示される", async ({ page }) => {
		const config = statusBarConfig({
			"plugin:fs|read_dir": rootDirEntries,
			get_file_at_ref: "# README\nHello world",
			get_staged_content: "# README\nHello world",
			"plugin:fs|read_text_file": "# README\nHello world",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Explorer に切り替えてファイルを開く
		await page.getByRole("button", { name: "Explorer" }).click();
		await page.getByText("README.md").first().click();
		await expect(
			page.locator(".flexlayout__tab_button", { hasText: "README.md" }),
		).toBeVisible({ timeout: 5000 });

		// StatusBar に言語 "Markdown"、エンコーディング "UTF-8" が表示される
		await expect(page.getByText("Markdown")).toBeVisible();
		await expect(page.getByText("UTF-8")).toBeVisible();
	});

	test("Agent状態がイベントで更新される", async ({ page }) => {
		const config = statusBarConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// agent-state-changed イベントを発火
		await emitTauriEvent(page, "agent-state-changed", {
			worktree_path: "/test/repo",
			state: "running",
		});

		// StatusBar に "Agent: running" が表示される
		await expect(page.getByText("Agent: running")).toBeVisible();
	});
});
