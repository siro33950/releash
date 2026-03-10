import { expect, test } from "@playwright/test";
import { buildMockConfig, rootDirEntries } from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * WorkflowView の統合テスト。
 * アプリ起動時のデフォルトcenterTabは"workflow"なので、
 * waitForApp直後はworkflowモードで表示される。
 */
function workflowConfig(overrides: Record<string, unknown> = {}) {
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

test.describe("Workflow View", () => {
	test("Workflowタブの上下分割レイアウトが表示される（シナリオ5）", async ({
		page,
	}) => {
		const config = workflowConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Workflowタブがアクティブである
		const workflowTab = page.getByRole("tab", { name: "Workflow" });
		await expect(workflowTab).toHaveAttribute("aria-selected", "true");

		// 上部にドキュメントビューア（Editor/Previewトグルボタンが存在）が表示される
		await expect(page.getByTitle("Editor")).toBeVisible();
		await expect(page.getByTitle("Preview")).toBeVisible();

		// 下部にMainAgentターミナルタブが存在する
		await expect(page.getByText("MainAgent")).toBeVisible();
	});

	test("TerminalTabPanelの初期タブ名がMainAgentである（シナリオ6）", async ({
		page,
	}) => {
		const config = workflowConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// "MainAgent" テキストがタブに表示されている
		await expect(page.getByText("MainAgent")).toBeVisible({ timeout: 5000 });
	});

	test("右サイドバー上部にPlan Timeline/Plan Commentsタブが表示される（シナリオ9）", async ({
		page,
	}) => {
		const config = workflowConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// workflowモードでPlan Timeline, Plan Commentsタブが存在
		await expect(
			page.getByRole("tab", { name: "Plan Timeline" }),
		).toBeVisible();
		await expect(
			page.getByRole("tab", { name: "Plan Comments" }),
		).toBeVisible();
	});

	test("右サイドバー下部にTimelineタブが表示される（シナリオ11）", async ({
		page,
	}) => {
		const config = workflowConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// workflowモードでTimelineタブが存在（"Plan Timeline"を除外するためexact指定）
		const reviewPanel = page.getByTestId("review");
		await expect(
			reviewPanel.getByRole("tab", { name: "Timeline", exact: true }),
		).toBeVisible();
	});

	test("右サイドバー折りたたみ時にWorkflowタブが操作可能（シナリオ19）", async ({
		page,
	}) => {
		const config = workflowConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Plan Timelineタブが表示されていることを確認（右サイドバーが開いている）
		await expect(
			page.getByRole("tab", { name: "Plan Timeline" }),
		).toBeVisible();

		// "Right Sidebar" トグルボタンをクリックして折りたたむ
		await page
			.getByRole("button", { name: "Toggle Right Sidebar" })
			.click();

		// Workflowのドキュメントビューア（Editor/Preview）が引き続き表示される
		await expect(page.getByRole("tab", { name: "Editor" })).toBeVisible();
		await expect(page.getByText("MainAgent")).toBeVisible();

		// 右サイドバーを再展開
		await page
			.getByRole("button", { name: "Toggle Right Sidebar" })
			.click();

		// タブ状態が復元される（Plan Timelineが表示される）
		await expect(
			page.getByRole("tab", { name: "Plan Timeline" }),
		).toBeVisible();
	});
});
