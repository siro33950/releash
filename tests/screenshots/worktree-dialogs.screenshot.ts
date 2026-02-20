import { expect, test } from "@playwright/test";
import {
	branchList,
	mixedChanges,
} from "../helpers/fixtures";
import {
	setupWorktreeView,
	xtermMask,
	monacoMask,
} from "../helpers/screenshot-utils";

test.describe("Worktree Dialogs", () => {
	test("branch creation dialog", async ({ page }) => {
		await setupWorktreeView(page, {
			list_branches: branchList,
			get_git_status: [],
		});
		// ブランチ作成ダイアログを開く（StatusBar or Sourceのボタンから）
		const branchBtn = page.getByText("feat/test").first();
		await branchBtn.click();
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-dialog-branch-create.png",
			{ mask: [...xtermMask(page), ...monacoMask(page)] },
		);
	});

	test("discard all changes dialog", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: mixedChanges,
			git_discard: null,
		});
		// Discard All ボタンを探す
		const discardAllBtn = page.getByTitle("Discard All Changes");
		await discardAllBtn.click();
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-dialog-discard-all.png",
			{ mask: [...xtermMask(page), ...monacoMask(page)] },
		);
	});

	test("git error dialog", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: mixedChanges,
			git_commit: {
				__mockError:
					"error: pathspec 'nonexistent' did not match any files",
			},
		});
		const summaryInput = page.getByPlaceholder("Commit summary");
		await summaryInput.fill("test commit");
		const commitBtn = page.getByRole("button", { name: "Commit" });
		await commitBtn.click();
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-dialog-git-error.png",
			{ mask: [...xtermMask(page), ...monacoMask(page)] },
		);
	});

	test("unsaved changes dialog", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": [
				{
					name: "test.txt",
					isDirectory: false,
					isFile: true,
					isSymlink: false,
				},
			],
			get_file_at_ref: "original content",
			get_staged_content: "",
		});
		// Explorer からファイルを開く
		const explorerBtn = page.getByRole("button", {
			name: "Explorer",
			exact: true,
		});
		await explorerBtn.click();
		await page.waitForTimeout(300);
		const file = page.getByText("test.txt").first();
		await file.click();
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-dialog-unsaved.png",
			{ mask: [...xtermMask(page), ...monacoMask(page)] },
		);
	});

	test("push error state", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: [],
			git_push: {
				__mockError:
					"error: failed to push some refs to 'origin'",
			},
		});
		const pushBtn = page.getByRole("button", { name: "Push" });
		await pushBtn.click();
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-dialog-push-error.png",
			{ mask: [...xtermMask(page), ...monacoMask(page)] },
		);
	});
});
