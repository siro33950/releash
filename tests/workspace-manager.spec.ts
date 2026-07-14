import { expect, test, type Locator } from "@playwright/test";
import {
	branchList,
	buildMockConfig,
	kanbanBranches,
} from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

async function waitForAnimations(locator: Locator) {
	await locator.evaluate(async (element) => {
		await Promise.all(
			element
				.getAnimations({ subtree: true })
				.map((animation) => animation.finished.catch(() => undefined)),
		);
	});
}

test.describe("Workspace Manager", () => {
	test("リポジトリが存在しない場合 No Repository が表示される", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_worktrees: [],
			list_branches_with_status: [],
			get_main_repo_path: { __mockError: "not a git repo" },
			get_repo_paths: [],
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await expect(page.getByText("No Repository")).toBeVisible();
		await expect(
			page.getByRole("button", { name: "Add Repository" }),
		).toBeVisible();
	});

	test("worktree 付きブランチが WorkspaceList に表示される", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// useWorktreeList は worktree_path != null のブランチのみ表示する
		await expect(page.getByTestId("worktree-item-feat/wip")).toBeVisible();
		await expect(
			page.getByTestId("worktree-item-feat/review"),
		).toBeVisible();
	});

	test("worktree 付きブランチをクリックするとツリーが折りたたまれる", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: [
				{
					kind: "session",
					id: "session-1",
					worktreePath: "/test/repo-worktrees/feat-wip",
					title: "Direct session",
					state: "active",
					updatedAt: 1000,
					workflowNodeSession: false,
					agentState: null,
				},
			],
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await expect(page.getByText("Direct session")).toBeVisible();
		await page.getByTestId("worktree-item-feat/wip").click();

		await expect(page.getByText("Direct session")).not.toBeVisible();
	});

	test("Workflow menu stays anchored to its trigger after hover out", async ({
		page,
	}) => {
		const worktreePath = "/test/repo-worktrees/feat-wip";
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches.filter(
				(branch) => branch.name === "feat/wip",
			),
			list_workspace_worktree_nodes: [
				{
					kind: "workflow",
					executionId: "execution-1",
					worktreePath,
					workflowName: "release",
					title: "Release workflow",
					status: "running",
					canStop: true,
					updatedAt: 1000,
					nodeExecutions: [
						{
							kind: "node",
							nodeExecutionId: "node-build-1",
							executionId: "execution-1",
							worktreePath,
							title: "build",
							nodeName: "build",
							status: "running",
							nodeKind: "session",
							nodeExecutionStatus: "running",
							canApprove: false,
							updatedAt: 1000,
							attempt: 1,
							sessions: [],
						},
					],
				},
			],
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByText("release", { exact: true }).hover();
		const trigger = page.getByRole("button", {
			name: "Open menu for release",
		});
		await expect(trigger).toBeVisible();
		const triggerBox = await trigger.boundingBox();
		expect(triggerBox).not.toBeNull();
		await trigger.click();

		const menu = page.getByRole("menu").filter({ hasText: "Stop" });
		await expect(menu).toBeVisible();
		await waitForAnimations(menu);
		const menuBox = await menu.boundingBox();
		expect(menuBox).not.toBeNull();
		expect(menuBox!.x).toBeGreaterThan(10);
		expect(menuBox!.y).toBeGreaterThan(10);
		expect(
			Math.abs(
				menuBox!.x + menuBox!.width / 2 - (triggerBox!.x + triggerBox!.width / 2),
			),
		).toBeLessThan(160);
		expect(
			Math.abs(menuBox!.y - (triggerBox!.y + triggerBox!.height)),
		).toBeLessThan(40);

		await page.mouse.move(700, 500);
		await expect(menu).toBeVisible();
		const afterHoverOutBox = await menu.boundingBox();
		expect(afterHoverOutBox).not.toBeNull();
		expect(afterHoverOutBox!.x).toBeGreaterThan(10);
		expect(afterHoverOutBox!.y).toBeGreaterThan(10);
		expect(Math.abs(afterHoverOutBox!.x - menuBox!.x)).toBeLessThan(2);
		expect(Math.abs(afterHoverOutBox!.y - menuBox!.y)).toBeLessThan(2);
	});
});

test.describe("CreateWorktreeModal", () => {
	test("Add worktree ボタンでモーダルが開く", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("Add worktree").click();

		await expect(
			page.getByRole("heading", { name: "New Worktree" }),
		).toBeVisible();
	});

	test("Branch タブでブランチ一覧がフィルタリングされる", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("Add worktree").click();
		await expect(page.getByRole("heading", { name: "New Worktree" })).toBeVisible();

		const dialog = page.getByRole("dialog");

		// Branch タブに切り替え
		await dialog.getByRole("tab", { name: "Branch" }).click();

		// worktree なしブランチが表示される
		await expect(dialog.getByText("feat/todo")).toBeVisible();
		await expect(dialog.getByText("feat/done")).toBeVisible();

		// フィルター入力
		await dialog.getByPlaceholder("Filter branches...").fill("done");

		// feat/done のみ表示
		await expect(dialog.getByText("feat/done")).toBeVisible();
		await expect(dialog.getByText("feat/todo")).not.toBeVisible();
	});

	test("Cancel ボタンでモーダルが閉じる", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("Add worktree").click();
		await expect(page.getByRole("heading", { name: "New Worktree" })).toBeVisible();

		await page.getByRole("button", { name: "Cancel" }).click();

		await expect(
			page.getByRole("heading", { name: "New Worktree" }),
		).not.toBeVisible();
	});

	test("Plain モードでブランチ名を入力すると Create が有効になる", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("Add worktree").click();
		await expect(page.getByRole("heading", { name: "New Worktree" })).toBeVisible();

		// Plain モードがデフォルト
		const branchInput = page.getByPlaceholder("feat/my-feature");
		await expect(branchInput).toBeVisible();

		// Create ボタンは初期状態で disabled
		const createBtn = page.getByRole("button", { name: "Create" });
		await expect(createBtn).toBeDisabled();

		// ブランチ名を入力
		await branchInput.fill("feat/new-feature");

		// Create ボタンが enabled になる
		await expect(createBtn).toBeEnabled();
	});
});
