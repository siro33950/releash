import { expect, test } from "@playwright/test";
import {
	prDetailChangesRequested,
	prDetailMerged,
	prDetailOpen,
} from "../helpers/fixtures";
import {
	setupWorktreeView,
	switchToView,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("Pull Request Panel", () => {
	test("no PR associated", async ({ page }) => {
		await setupWorktreeView(page, {
			get_pr_detail: null,
		});
		await switchToView(page, "Pull Request");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-pull-request-none.png",
			{ mask: xtermMask(page) },
		);
	});

	test("PR open state", async ({ page }) => {
		await setupWorktreeView(page, {
			get_pr_detail: prDetailOpen,
			get_cached_pr_status: {
				open_prs: {
					"feat/test": {
						number: 42,
						url: "https://github.com/test/repo/pull/42",
					},
				},
				merged_branches: [],
			},
		});
		await switchToView(page, "Pull Request");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-pull-request-open.png",
			{ mask: xtermMask(page) },
		);
	});

	test("PR merged state", async ({ page }) => {
		await setupWorktreeView(page, {
			get_pr_detail: prDetailMerged,
			get_cached_pr_status: {
				open_prs: {
					"feat/test": {
						number: 42,
						url: "https://github.com/test/repo/pull/42",
					},
				},
				merged_branches: [],
			},
		});
		await switchToView(page, "Pull Request");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-pull-request-merged.png",
			{ mask: xtermMask(page) },
		);
	});

	test("PR changes requested state", async ({ page }) => {
		await setupWorktreeView(page, {
			get_pr_detail: prDetailChangesRequested,
			get_cached_pr_status: {
				open_prs: {
					"feat/test": {
						number: 42,
						url: "https://github.com/test/repo/pull/42",
					},
				},
				merged_branches: [],
			},
		});
		await switchToView(page, "Pull Request");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-pull-request-changes-requested.png",
			{ mask: xtermMask(page) },
		);
	});

	test("PR fetch error", async ({ page }) => {
		await setupWorktreeView(page, {
			get_pr_detail: {
				__mockError: "Failed to fetch PR details: network error",
			},
			get_cached_pr_status: {
				open_prs: {
					"feat/test": {
						number: 42,
						url: "https://github.com/test/repo/pull/42",
					},
				},
				merged_branches: [],
			},
		});
		await switchToView(page, "Pull Request");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-pull-request-error.png",
			{ mask: xtermMask(page) },
		);
	});
});
