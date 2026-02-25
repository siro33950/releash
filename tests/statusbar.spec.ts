import { expect, test } from "@playwright/test";
import { buildMockConfig } from "./helpers/fixtures";
import { setupTauriMock, emitTauriEvent } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * WorkspaceList のブランチ名表示および Agent 状態バッジのテスト。
 * worktree が1つ存在する状態で各表示・イベント連動を検証する。
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

	test("Agent状態がイベントで更新される", async ({ page }) => {
		const config = statusBarConfig({
			list_branches_with_status: [
				{
					name: "feat/test",
					is_default: false,
					worktree_path: "/test/repo",
					dirty_count: 0,
					is_merged: false,
					has_pr: false,
					pr_number: null,
					pr_url: null,
					ahead: 0,
					behind: 0,
					has_upstream: true,
					base_ahead: 0,
				},
			],
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// agent-state-changed イベントを発火
		await emitTauriEvent(page, "agent-state-changed", {
			worktree_path: "/test/repo",
			state: "running",
		});

		// WorkspaceListのブランチアイテムに "Running" バッジが表示される
		await expect(page.getByText("Running")).toBeVisible({ timeout: 5000 });
	});
});
