import { expect, test } from "@playwright/test";
import { buildMockConfig, kanbanBranches } from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * StatusBar のブランチ名表示と workspace 状態イベント購読のテスト。
 * worktree が1つ存在する状態で表示とイベント処理を検証する。
 */
function statusBarConfig(overrides: Record<string, unknown> = {}) {
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
		"current-branch": "feat/my-branch",
		get_git_status: [],
		...overrides,
	});
}

test.describe("StatusBar", () => {
	test("購読からworkspaceの表示が更新され取り直しを呼ばない", async ({ page }) => {
		const client = await setupTauriMock(
			page,
			statusBarConfig({
				"workspaceBranches": kanbanBranches.filter(
					(branch) => branch.name === "feat/wip",
				),
				"workspaceTree": {
					nodes: [],
					archivedSessions: [],
					preferredNodeId: null,
				},
			}),
		);
		await waitForApp(page);
		await expect(page.getByText("feat/my-branch")).toBeVisible();
		await expect(page.getByText("No sessions or workflows")).toBeVisible();
		await page.evaluate(() =>
			window.__RELEASH_BACKEND__?.setWorkspaceTree(
				{
					nodes: [
						{
							kind: "node",
							processPresence: "unknown",
							id: "ws-node",
							title: "From ws push",
							status: "active",
							contentKind: "session",
							capabilities: {
								canRename: false,
								canApprove: false,
								canRetry: false,
								canResumeSession: false,
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
        await client.refreshStates();
        expect(client.clientRequests.some((request) => request.command === "refresh_workspaces")).toBe(false);
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
        expect(client.stateRequests).toContain(JSON.stringify(["current-branch", ["/test/repo"]]));
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
			statusBarConfig({ "current-branch": { __mockError: "failed" } }),
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
			"workspaceBranches": [
				{
					name: "feat/test",
					is_main_worktree: false,
					is_deleting: false,
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
		const client = await setupTauriMock(page, config);
		await waitForApp(page);

        await client.refreshStates();

		await expect(page.getByText("feat/my-branch")).toBeVisible();
		await expect(
			page.getByRole("heading", { name: "Something went wrong" }),
		).not.toBeVisible();
	});
});
