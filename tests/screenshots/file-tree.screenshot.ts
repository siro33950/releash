import { expect, test } from "@playwright/test";
import {
	fsPluginCommands,
	rootDirEntries,
} from "../helpers/fixtures";
import {
	setupWorktreeView,
	switchToView,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("File Tree Panel", () => {
	test("root directory listing", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-root.png",
			{ mask: xtermMask(page) },
		);
	});

	test("expanded directory", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			...fsPluginCommands,
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		// src ディレクトリをクリックして展開
		const srcDir = page.getByText("src").first();
		await srcDir.click();
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-expanded.png",
			{ mask: xtermMask(page) },
		);
	});

	test("selected file", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		// ファイルを選択
		const file = page.getByText("README.md").first();
		await file.click();
		await page.waitForTimeout(200);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-selected.png",
			{ mask: xtermMask(page) },
		);
	});

	test("new file input", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			...fsPluginCommands,
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		const newFileBtn = page.getByTitle("New File");
		await newFileBtn.click();
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-new-file.png",
			{ mask: xtermMask(page) },
		);
	});

	test("new folder input", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			...fsPluginCommands,
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		const newFolderBtn = page.getByTitle("New Folder");
		await newFolderBtn.click();
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-new-folder.png",
			{ mask: xtermMask(page) },
		);
	});

	test("delete confirmation dialog", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			...fsPluginCommands,
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		const file = page.getByText("README.md").first();
		await file.click({ button: "right" });
		await page.waitForTimeout(200);
		const deleteItem = page.getByText("Delete").first();
		await deleteItem.click();
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-delete-dialog.png",
			{ mask: xtermMask(page) },
		);
	});

	test("context menu on file", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			...fsPluginCommands,
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		const file = page.getByText("README.md").first();
		await file.click({ button: "right" });
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-context-menu-file.png",
			{ mask: xtermMask(page) },
		);
	});

	test("context menu on folder", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			...fsPluginCommands,
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		const folder = page.getByText("src").first();
		await folder.click({ button: "right" });
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-context-menu-folder.png",
			{ mask: xtermMask(page) },
		);
	});

	test("empty directory", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": [],
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-file-tree-empty.png",
			{ mask: xtermMask(page) },
		);
	});
});
