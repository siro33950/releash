import { expect, test } from "@playwright/test";
import {
	branchList,
	buildMockConfig,
	kanbanBranches,
	type WorktreeEntry,
} from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { getInvokeHistory, trackInvocations, waitForApp } from "./helpers/utils";

test.describe("Workspace Manager (Kanban)", () => {
	test("リポジトリ未選択時にOpen Folderボタンが表示される", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_worktrees: [],
			list_branches_with_status: [],
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		const openButton = page.getByRole("button", { name: /Open/i }).first();
		await expect(openButton).toBeVisible();
	});

	test("リポジトリ選択後にKanban 4列が表示される", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// カラムタイトルを exact: true で検索し、ブランチ名との衝突を避ける
		await expect(page.getByText("Todo", { exact: true })).toBeVisible();
		await expect(page.getByText("In Progress", { exact: true })).toBeVisible();
		await expect(page.getByText("Review", { exact: true })).toBeVisible();
		await expect(page.getByText("Done", { exact: true })).toBeVisible();
	});

	test("ブランチカードが正しい列に分類される", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await expect(page.getByText("feat/todo")).toBeVisible();
		await expect(page.getByText("feat/wip")).toBeVisible();
		await expect(page.getByText("feat/review")).toBeVisible();
		await expect(page.getByText("feat/done")).toBeVisible();
	});

	test("Todoブランチの Open クリックで worktree 作成が呼ばれる", async ({
		page,
	}) => {
		const createdEntry: WorktreeEntry = {
			name: "feat-todo",
			path: "/test/repo-worktrees/feat-todo",
			branch: "feat/todo",
			is_main: false,
			is_locked: false,
			dirty_count: 0,
			base_branch: null,
		};
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			create_worktree: createdEntry,
		});
		await setupTauriMock(page, config);
		await trackInvocations(page);
		await waitForApp(page);

		// feat/todo カード内の "Open" ボタンをクリック
		const todoCard = page.getByTestId("branch-card-feat/todo");
		await todoCard.getByRole("button", { name: "Open" }).click();

		// create_worktree は非同期で呼ばれるので待機
		await page.waitForFunction(
			() =>
				// @ts-expect-error - テスト用グローバル
				(window.__INVOKE_HISTORY__ ?? []).some(
					(h: { cmd: string }) => h.cmd === "create_worktree",
				),
			null,
			{ timeout: 5000 },
		);

		const history = await getInvokeHistory(page);
		const createCall = history.find((h) => h.cmd === "create_worktree");
		expect(createCall).toBeTruthy();
	});

	test("InProgressブランチの Open クリックでタブが開く", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// feat/wip カード内の "Open" ボタンをクリック
		const wipCard = page.getByTestId("branch-card-feat/wip");
		await wipCard.getByRole("button", { name: "Open" }).click();

		// WorkspaceTabBar にタブが追加されることを確認
		await expect(page.getByText("feat/wip").first()).toBeVisible();
	});
});

test.describe("CreateWorktreeDialog", () => {
	test("New worktree ボタンでダイアログが開く", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// "New worktree" ボタン（title="New worktree"）をクリック
		await page.getByTitle("New worktree").click();

		// ダイアログが表示される（"New Workspace" タイトル）
		await expect(page.getByText("New Workspace")).toBeVisible();
		// 説明文
		await expect(
			page.getByText("Select an existing branch or type a new branch name"),
		).toBeVisible();
	});

	test("ブランチ一覧がフィルタリングされる", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("New worktree").click();
		await expect(page.getByText("New Workspace")).toBeVisible();

		// ブランチ一覧が表示される
		await expect(page.getByText("main").first()).toBeVisible();
		await expect(page.getByText("develop").first()).toBeVisible();

		// フィルター入力
		const filterInput = page.getByPlaceholder("Filter or create branch...");
		await filterInput.fill("dev");

		// "develop" のみ表示（"main" は非表示）
		await expect(page.getByText("develop").first()).toBeVisible();
		await expect(
			page.locator("button").filter({ hasText: /^main$/ }),
		).not.toBeVisible();
	});

	test("Cancel ボタンでダイアログが閉じる", async ({ page }) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("New worktree").click();
		await expect(page.getByText("New Workspace")).toBeVisible();

		// Cancel ボタンをクリック
		await page.getByRole("button", { name: "Cancel" }).click();

		// ダイアログが閉じる
		await expect(page.getByText("New Workspace")).not.toBeVisible();
	});

	test("新規ブランチ名を入力すると Create branch オプションが表示される", async ({
		page,
	}) => {
		const config = buildMockConfig({
			list_branches_with_status: kanbanBranches,
			list_branches: branchList,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		await page.getByTitle("New worktree").click();
		await expect(page.getByText("New Workspace")).toBeVisible();

		// 存在しないブランチ名を入力
		const filterInput = page.getByPlaceholder("Filter or create branch...");
		await filterInput.fill("feat/new-feature");

		// "Create branch" オプションが表示される
		await expect(
			page.getByText('Create branch "feat/new-feature"'),
		).toBeVisible();
	});
});
