import { expect, test } from "@playwright/test";
import {
	branchList,
	issueList,
	kanbanBranchesFull,
	notionConfig,
	notionLabelOptions,
	notionTasks,
} from "../helpers/fixtures";
import {
	setupWorkspaceManager,
	switchToView,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("Workspace Panels", () => {
	// -------------------------------------------------------
	// IssuePanel
	// -------------------------------------------------------
	test.describe("Issue Panel", () => {
		test("empty issue list", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_cached_issues: [],
				fetch_issues: [],
			});
			await switchToView(page, "Issues");
			await page.waitForTimeout(500);
			await expect(page).toHaveScreenshot(
				"workspace-issues-empty.png",
				{ mask: xtermMask(page) },
			);
		});

		test("issue list with items", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_cached_issues: issueList,
				fetch_issues: issueList,
			});
			await switchToView(page, "Issues");
			await page.waitForTimeout(500);
			await expect(page).toHaveScreenshot(
				"workspace-issues-list.png",
				{ mask: xtermMask(page) },
			);
		});

		test("issue filter by label", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_cached_issues: issueList,
				fetch_issues: issueList,
			});
			await switchToView(page, "Issues");
			await page.waitForTimeout(500);
			// ラベルフィルタをクリック（存在する場合）
			const filterInput = page.getByPlaceholder(/search|filter/i).first();
			if (await filterInput.isVisible()) {
				await filterInput.fill("bug");
				await page.waitForTimeout(300);
			}
			await expect(page).toHaveScreenshot(
				"workspace-issues-filter-label.png",
				{ mask: xtermMask(page) },
			);
		});

		test("issue filter by milestone", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_cached_issues: issueList,
				fetch_issues: issueList,
			});
			await switchToView(page, "Issues");
			await page.waitForTimeout(500);
			const filterInput = page.getByPlaceholder(/search|filter/i).first();
			if (await filterInput.isVisible()) {
				await filterInput.fill("v0.2.0");
				await page.waitForTimeout(300);
			}
			await expect(page).toHaveScreenshot(
				"workspace-issues-filter-milestone.png",
				{ mask: xtermMask(page) },
			);
		});

		test("issue create worktree button", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_cached_issues: issueList,
				fetch_issues: issueList,
				list_branches: branchList,
			});
			await switchToView(page, "Issues");
			await page.waitForTimeout(500);
			// 最初のIssueカードのCreate Worktreeボタン
			const createBtn = page
				.getByRole("button", { name: /Create.*Worktree|Open/i })
				.first();
			if (await createBtn.isVisible()) {
				await createBtn.click();
				await page.waitForTimeout(300);
			}
			await expect(page).toHaveScreenshot(
				"workspace-issues-create-worktree.png",
				{ mask: xtermMask(page) },
			);
		});

		test("issue open worktree action", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_cached_issues: issueList,
				fetch_issues: issueList,
				list_branches: branchList,
			});
			await switchToView(page, "Issues");
			await page.waitForTimeout(500);
			await expect(page).toHaveScreenshot(
				"workspace-issues-with-actions.png",
				{ mask: xtermMask(page) },
			);
		});
	});

	// -------------------------------------------------------
	// NotionPanel
	// -------------------------------------------------------
	test.describe("Notion Panel", () => {
		test("not configured", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_notion_config: null,
			});
			await switchToView(page, "Notion Tasks");
			await page.waitForTimeout(500);
			await expect(page).toHaveScreenshot(
				"workspace-notion-not-configured.png",
				{ mask: xtermMask(page) },
			);
		});

		test("configuration form", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_notion_config: null,
			});
			await switchToView(page, "Notion Tasks");
			await page.waitForTimeout(500);
			// 設定ボタンをクリック
			const configBtn = page
				.getByRole("button", { name: /Configure|Setup|Connect/i })
				.first();
			if (await configBtn.isVisible()) {
				await configBtn.click();
				await page.waitForTimeout(300);
			}
			await expect(page).toHaveScreenshot(
				"workspace-notion-config-form.png",
				{ mask: xtermMask(page) },
			);
		});

		test("task list with items", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_notion_config: notionConfig,
				query_notion_tasks: {
					tasks: notionTasks,
					has_more: false,
					next_cursor: null,
				},
				fetch_notion_label_options: notionLabelOptions,
			});
			await switchToView(page, "Notion Tasks");
			await page.waitForTimeout(500);
			await expect(page).toHaveScreenshot(
				"workspace-notion-task-list.png",
				{ mask: xtermMask(page) },
			);
		});

		test("empty task list", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_notion_config: notionConfig,
				query_notion_tasks: {
					tasks: [],
					has_more: false,
					next_cursor: null,
				},
				fetch_notion_label_options: notionLabelOptions,
			});
			await switchToView(page, "Notion Tasks");
			await page.waitForTimeout(500);
			await expect(page).toHaveScreenshot(
				"workspace-notion-empty-tasks.png",
				{ mask: xtermMask(page) },
			);
		});
	});

	// -------------------------------------------------------
	// RemotePanel
	// -------------------------------------------------------
	test.describe("Remote Panel", () => {
		test("server stopped", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_server_info: null,
			});
			await switchToView(page, "Remote");
			await page.waitForTimeout(500);
			await expect(page).toHaveScreenshot(
				"workspace-remote-stopped.png",
				{ mask: xtermMask(page) },
			);
		});

		test("no repo available", async ({ page }) => {
			await setupWorkspaceManager(page, {
				get_server_info: null,
				get_network_info: [],
			});
			await switchToView(page, "Remote");
			await page.waitForTimeout(500);
			await expect(page).toHaveScreenshot(
				"workspace-remote-no-repo.png",
				{ mask: xtermMask(page) },
			);
		});
	});

	// -------------------------------------------------------
	// Settings (WorkspaceManager context)
	// -------------------------------------------------------
	test("workspace settings panel", async ({ page }) => {
		await setupWorkspaceManager(page);
		await switchToView(page, "Settings");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"workspace-settings.png",
			{ mask: xtermMask(page) },
		);
	});

	// -------------------------------------------------------
	// CreateWorktreeDialog (from Kanban)
	// -------------------------------------------------------
	test("create worktree dialog from kanban", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: kanbanBranchesFull,
			list_branches: branchList,
			get_cached_pr_status: {
				open_prs: {},
				merged_branches: [],
			},
		});
		await page.waitForTimeout(500);
		// Kanban の "Create Worktree" ボタンをクリック
		const createBtn = page
			.getByRole("button", { name: /Create.*Worktree|New Worktree|\+/i })
			.first();
		if (await createBtn.isVisible()) {
			await createBtn.click();
			await page.waitForTimeout(300);
		}
		await expect(page).toHaveScreenshot(
			"workspace-create-worktree-dialog.png",
			{ mask: xtermMask(page) },
		);
	});
});
