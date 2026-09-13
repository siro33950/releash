import { expect, test } from "@playwright/test";
import { buildMockConfig, kanbanBranches } from "./helpers/fixtures";
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
			},
		],
		get_current_branch: "feat/my-branch",
		get_git_status: [],
		...overrides,
	});
}

test.describe("StatusBar", () => {
	test("workflowのws pushでworkspaceの表示が更新される", async ({ page }) => {
		const client = await setupTauriMock(
			page,
			statusBarConfig({
				list_branches_with_status: kanbanBranches.filter(
					(branch) => branch.name === "feat/wip",
				),
				list_workspace_worktree_nodes: {
					nodes: [],
					archivedSessions: [],
					preferredNodeId: null,
				},
			}),
		);
		await waitForApp(page);
		await expect(page.getByText("feat/my-branch")).toBeVisible();
		await page.evaluate(() =>
			window.__TAURI_INTERNALS__?.setMockResponse(
				"list_workspace_worktree_nodes",
				{
					nodes: [
						{
							kind: "node",
							id: "ws-node",
							title: "From ws push",
							status: "active",
							contentKind: "session",
							capabilities: {
								canRename: false,
								canApprove: false,
								canRetry: false,
								canClose: false,
							},
							pastAttempts: [],
							pastAttemptsCollapsed: false,
							updatedAt: 2,
						},
					],
					archivedSessions: [],
					preferredNodeId: null,
				},
			),
		);
		client.push("workflow-execution-changed", {
			worktreePath: "/test/repo-worktrees/feat-wip",
			workflowExecution: {
				id: "execution-1",
				workflowName: "test",
				status: "running",
				currentNode: null,
				worktreePath: "/test/repo-worktrees/feat-wip",
				createdFrom: "desktop_ui",
				startedAt: 1,
				updatedAt: 2,
				completedAt: null,
				errorReason: null,
				interruptionReason: null,
				resumeFromNode: null,
				totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
				nodeExecutions: [],
				artifacts: [],
				fanouts: [],
				approvalTarget: null,
			},
		});
		await expect(page.getByText("From ws push")).toBeVisible();
		expect(
			await page.evaluate(() =>
				window.__TAURI_INTERNALS__?.ipcInvocations.some(
					({ cmd, args }) =>
						cmd === "plugin:event|listen" &&
						args.event === "workflow-execution-changed",
				),
			),
		).toBe(false);
	});

	test("ブランチ名が表示される", async ({ page }) => {
		const config = statusBarConfig();
		const client = await setupTauriMock(page, config);
		await waitForApp(page);

		await expect(page.getByText("feat/my-branch")).toBeVisible();
		expect(client.clientRequests).toContainEqual({
			request_id: expect.any(String),
			command: "get_current_branch",
			args: { repoPath: "/test/repo" },
		});
		expect(
			await page.evaluate(() =>
				window.__TAURI_INTERNALS__?.ipcInvocations.some(
					({ cmd }) => cmd === "get_current_branch",
				),
			),
		).toBe(false);
	});

	test("ws取得失敗時はブランチ名を表示せずTauriへfallbackしない", async ({
		page,
	}) => {
		const client = await setupTauriMock(
			page,
			statusBarConfig({ get_current_branch: { __mockError: "failed" } }),
		);
		await waitForApp(page);
		await expect.poll(() => client.clientRequests.length).toBeGreaterThan(0);
		await expect(page.getByText("feat/my-branch")).not.toBeVisible();
		expect(
			await page.evaluate(() =>
				window.__TAURI_INTERNALS__?.ipcInvocations.some(
					({ cmd }) => cmd === "get_current_branch",
				),
			),
		).toBe(false);
	});

	test("workspace 状態イベントを受けてもレイアウトが維持される", async ({
		page,
	}) => {
		const config = statusBarConfig({
			list_branches_with_status: [
				{
					name: "feat/test",
					is_main_worktree: false,
					worktree_path: "/test/repo",
					dirty_count: 0,
					is_merged: false,
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
