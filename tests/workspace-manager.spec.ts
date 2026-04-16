import { expect, test } from "@playwright/test";
import {
	branchList,
	buildMockConfig,
	kanbanBranches,
} from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

test.describe("Workspace Manager", () => {
	test("リポジトリが存在しない場合 No repositories が表示される", async ({
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

		await expect(page.getByText("No repositories")).toBeVisible();
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

	test("worktree 付きブランチをクリックすると WorktreeView が開く", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// feat/wip（worktree_path あり）をクリック
		await page.getByTestId("worktree-item-feat/wip").click();

		// WorktreeView 固有の comments パネルが表示される
		await expect(page.getByTestId("comments")).toBeVisible({
			timeout: 5000,
		});
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
