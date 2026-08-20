import { expect, test } from "@playwright/test";
import { buildMockConfig } from "./helpers/fixtures";
import { setupTauriMock, emitTauriEvent } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * StatusBar のブランチ名表示と workspace 状態イベント購読のテスト。
 * worktree が1つ存在する状態で表示とイベント処理を検証する。
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
				management_kind: "working_area",
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

	test("workspace 状態イベントを受けてもレイアウトが維持される", async ({
		page,
	}) => {
		const config = statusBarConfig({
			list_branches_with_status: [
				{
					name: "feat/test",
					is_default: false,
					worktree_path: "/test/repo",
					management_kind: "working_area",
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

		// workspace-status-changed イベントを発火（Rust 中央管理からの通知）
		await emitTauriEvent(page, "workspace-status-changed", {
			worktree_id: "/test/repo",
			worktree_path: "/test/repo",
			aggregated_state: "running",
			running_count: 1,
			waiting_count: 0,
			error_count: 0,
			session_count: 1,
			last_activity_at: 1000,
		});

		await expect(page.getByText("feat/my-branch")).toBeVisible();
		await expect(
			page.getByRole("heading", { name: "Something went wrong" }),
		).not.toBeVisible();
	});
});
