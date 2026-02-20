import { expect, test } from "@playwright/test";
import {
	kanbanBranches,
	kanbanBranchesFull,
} from "../helpers/fixtures";
import {
	setupWorkspaceManager,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("Workspace Kanban Board", () => {
	test("empty board", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [],
		});
		await expect(page).toHaveScreenshot(
			"workspace-kanban-empty-board.png",
			{ mask: xtermMask(page) },
		);
	});

	test("full board with all column types", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: kanbanBranchesFull,
			get_cached_pr_status: {
				open_prs: {
					"feat/in-review": {
						number: 88,
						url: "https://github.com/test/repo/pull/88",
					},
				},
				merged_branches: ["feat/completed"],
			},
		});
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-full-board.png",
			{ mask: xtermMask(page) },
		);
	});

	test("card - local branch without worktree", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [kanbanBranches[0]], // feat/todo
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-card-local.png",
			{ mask: xtermMask(page) },
		);
	});

	test("card - remote only branch", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [kanbanBranchesFull[1]], // feat/remote-only
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-card-remote-only.png",
			{ mask: xtermMask(page) },
		);
	});

	test("card - with PR badge", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [kanbanBranches[2]], // feat/review with PR
			get_cached_pr_status: {
				open_prs: {
					"feat/review": {
						number: 42,
						url: "https://github.com/test/repo/pull/42",
					},
				},
				merged_branches: [],
			},
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-card-pr.png",
			{ mask: xtermMask(page) },
		);
	});

	test("card - merged branch", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [kanbanBranches[3]], // feat/done (merged)
			get_cached_pr_status: {
				open_prs: {},
				merged_branches: ["feat/done"],
			},
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-card-merged.png",
			{ mask: xtermMask(page) },
		);
	});

	test("card - agent running state", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [kanbanBranchesFull[3]], // agent_state: running
			get_agent_states: {
				"feat/agent-running": {
					state: "running",
					timestamp: 9999999999,
				},
			},
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-card-agent-running.png",
			{ mask: xtermMask(page) },
		);
	});

	test("card - agent done state", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [kanbanBranchesFull[4]], // agent_state: done
			get_agent_states: {
				"feat/agent-done": {
					state: "done",
					timestamp: 9999999999,
				},
			},
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-card-agent-done.png",
			{ mask: xtermMask(page) },
		);
	});

	test("card - dirty with ahead/behind badges", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [kanbanBranchesFull[2]], // dirty:5, ahead:3, behind:1
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-card-dirty-ahead-behind.png",
			{ mask: xtermMask(page) },
		);
	});

	test("card - active worktree with dirty count", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: [kanbanBranches[1]], // feat/wip dirty:2
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-card-active-dirty.png",
			{ mask: xtermMask(page) },
		);
	});

	test("board with basic branches", async ({ page }) => {
		await setupWorkspaceManager(page, {
			list_branches_with_status: kanbanBranches,
			get_cached_pr_status: {
				open_prs: {
					"feat/review": {
						number: 42,
						url: "https://github.com/test/repo/pull/42",
					},
				},
				merged_branches: ["feat/done"],
			},
		});
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-basic-board.png",
			{ mask: xtermMask(page) },
		);
	});

	test("multiple agent states on board", async ({ page }) => {
		const agentBranches = [
			{
				...kanbanBranchesFull[3],
				name: "feat/agent-1",
				agent_state: "running" as const,
			},
			{
				...kanbanBranchesFull[4],
				name: "feat/agent-2",
				agent_state: "done" as const,
			},
			{
				...kanbanBranchesFull[2],
				name: "feat/agent-3",
				agent_state: "error" as const,
			},
			{
				...kanbanBranchesFull[3],
				name: "feat/agent-4",
				agent_state: "waiting" as const,
			},
		];
		await setupWorkspaceManager(page, {
			list_branches_with_status: agentBranches,
			get_agent_states: {
				"feat/agent-1": { state: "running", timestamp: 9999999999 },
				"feat/agent-2": { state: "done", timestamp: 9999999999 },
				"feat/agent-3": { state: "error", timestamp: 9999999999 },
				"feat/agent-4": { state: "waiting", timestamp: 9999999999 },
			},
		});
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"workspace-kanban-multi-agent.png",
			{ mask: xtermMask(page) },
		);
	});
});
