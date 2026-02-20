import { expect, test } from "@playwright/test";
import {
	mixedChanges,
	stagedChanges,
	unstagedChanges,
} from "../helpers/fixtures";
import {
	setupWorktreeView,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("Source Control Panel", () => {
	test("empty state", async ({ page }) => {
		await setupWorktreeView(page, { get_git_status: [] });
		await expect(page).toHaveScreenshot("worktree-source-control-empty.png", {
			mask: xtermMask(page),
		});
	});

	test("unstaged files only", async ({ page }) => {
		await setupWorktreeView(page, { get_git_status: unstagedChanges });
		await expect(page).toHaveScreenshot(
			"worktree-source-control-unstaged.png",
			{ mask: xtermMask(page) },
		);
	});

	test("staged files only", async ({ page }) => {
		await setupWorktreeView(page, { get_git_status: stagedChanges });
		await expect(page).toHaveScreenshot(
			"worktree-source-control-staged.png",
			{ mask: xtermMask(page) },
		);
	});

	test("mixed staged and unstaged", async ({ page }) => {
		await setupWorktreeView(page, { get_git_status: mixedChanges });
		await expect(page).toHaveScreenshot(
			"worktree-source-control-mixed.png",
			{ mask: xtermMask(page) },
		);
	});

	test("commit message input", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: stagedChanges,
			git_commit: "abc1234",
		});
		const summaryInput = page.getByPlaceholder("Commit summary");
		await summaryInput.fill("feat: add new feature");
		await expect(page).toHaveScreenshot(
			"worktree-source-control-commit-input.png",
			{ mask: xtermMask(page) },
		);
	});

	test("long commit message with description", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: stagedChanges,
			git_commit: "abc1234",
		});
		const summaryInput = page.getByPlaceholder("Commit summary");
		await summaryInput.fill(
			"feat: this is a very long commit message that might overflow the input field",
		);
		const descInput = page.getByPlaceholder("Description (optional)");
		if (await descInput.isVisible()) {
			await descInput.fill(
				"This is a detailed description of the changes.\n- Changed file A\n- Updated module B",
			);
		}
		await expect(page).toHaveScreenshot(
			"worktree-source-control-long-message.png",
			{ mask: xtermMask(page) },
		);
	});

	test("commit description expanded", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: stagedChanges,
			git_commit: "abc1234",
		});
		const descInput = page.getByPlaceholder("Description (optional)");
		if (await descInput.isVisible()) {
			await descInput.fill("Detailed commit description here.");
		}
		await expect(page).toHaveScreenshot(
			"worktree-source-control-description.png",
			{ mask: xtermMask(page) },
		);
	});

	test("commit error state", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: stagedChanges,
			git_commit: { __mockError: "nothing to commit, working tree clean" },
		});
		const summaryInput = page.getByPlaceholder("Commit summary");
		await summaryInput.fill("test commit");
		const commitBtn = page.getByRole("button", { name: "Commit" });
		await commitBtn.click();
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-source-control-error.png",
			{ mask: xtermMask(page) },
		);
	});

	test("discard changes dialog", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: unstagedChanges,
			git_discard: null,
		});
		// FileStatusItem のコンテキストメニューまたは discard ボタン
		const discardAllBtn = page.getByTitle("Discard All Changes");
		if (await discardAllBtn.isVisible()) {
			await discardAllBtn.click();
			await page.waitForTimeout(300);
		}
		await expect(page).toHaveScreenshot(
			"worktree-source-control-discard-dialog.png",
			{ mask: xtermMask(page) },
		);
	});

	test("context menu on file", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: unstagedChanges,
			git_stage: null,
			git_discard: null,
		});
		const fileItem = page.getByText("App.tsx").first();
		await fileItem.click({ button: "right" });
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-source-control-context-menu.png",
			{ mask: xtermMask(page) },
		);
	});
});
