import { expect, test } from "@playwright/test";
import {
	rootDirEntries,
	mixedChanges,
	searchResults,
} from "../helpers/fixtures";
import {
	setupWorktreeView,
	switchToView,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("Editor Layout", () => {
	test("empty editor state", async ({ page }) => {
		await setupWorktreeView(page);
		await expect(page).toHaveScreenshot(
			"worktree-editor-empty.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("file opened in editor", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			get_file_at_ref: "# Test File\n\nThis is a test file content.",
			get_staged_content: "",
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		const file = page.getByText("README.md").first();
		await file.click();
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-editor-file-open.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("multiple tabs open", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			get_file_at_ref: "content",
			get_staged_content: "",
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		// 複数ファイルを開く
		const readme = page.getByText("README.md").first();
		await readme.click();
		await page.waitForTimeout(300);
		const pkg = page.getByText("package.json").first();
		await pkg.click();
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-editor-multiple-tabs.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("full layout with all panels visible", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: mixedChanges,
		});
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-editor-full-layout.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("sidebar collapsed", async ({ page }) => {
		await setupWorktreeView(page);
		// サイドバーの折りたたみトグル
		const toggleSidebar = page.getByTitle("Toggle Sidebar").first();
		if (await toggleSidebar.isVisible()) {
			await toggleSidebar.click();
			await page.waitForTimeout(300);
		}
		await expect(page).toHaveScreenshot(
			"worktree-editor-sidebar-collapsed.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("terminal collapsed", async ({ page }) => {
		await setupWorktreeView(page);
		// ターミナルの折りたたみトグル
		const toggleTerminal = page.getByTitle("Toggle Terminal").first();
		if (await toggleTerminal.isVisible()) {
			await toggleTerminal.click();
			await page.waitForTimeout(300);
		}
		await expect(page).toHaveScreenshot(
			"worktree-editor-terminal-collapsed.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("all panels collapsed", async ({ page }) => {
		await setupWorktreeView(page);
		const toggleSidebar = page.getByTitle("Toggle Sidebar").first();
		if (await toggleSidebar.isVisible()) {
			await toggleSidebar.click();
			await page.waitForTimeout(200);
		}
		const toggleTerminal = page.getByTitle("Toggle Terminal").first();
		if (await toggleTerminal.isVisible()) {
			await toggleTerminal.click();
			await page.waitForTimeout(200);
		}
		await expect(page).toHaveScreenshot(
			"worktree-editor-all-collapsed.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("source control with search panel side by side", async ({ page }) => {
		await setupWorktreeView(page, {
			get_git_status: mixedChanges,
			search_files: searchResults,
		});
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-editor-source-control-view.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("search view layout", async ({ page }) => {
		await setupWorktreeView(page, {
			search_files: searchResults,
		});
		await switchToView(page, "Search");
		await page.waitForTimeout(300);
		const searchInput = page.getByPlaceholder("Search");
		await searchInput.fill("useState");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-editor-search-view.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("pull request view layout", async ({ page }) => {
		await setupWorktreeView(page, {
			get_pr_detail: null,
		});
		await switchToView(page, "Pull Request");
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-editor-pr-view.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("agent tab view", async ({ page }) => {
		await setupWorktreeView(page);
		// 中央パネルの Agent タブに切り替え
		const agentTab = page.getByRole("tab", { name: /Agent/i });
		await agentTab.click();
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-editor-agent-tab.png",
			{ mask: [...xtermMask(page)] },
		);
	});

	test("editor with diff toolbar", async ({ page }) => {
		await setupWorktreeView(page, {
			"plugin:fs|read_dir": rootDirEntries,
			get_git_status: mixedChanges,
			get_file_at_ref: "original content\nline 2\nline 3",
			get_staged_content: "",
		});
		await switchToView(page, "Explorer");
		await page.waitForTimeout(300);
		const file = page.getByText("README.md").first();
		await file.click();
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-editor-diff-toolbar.png",
			{ mask: [...xtermMask(page)] },
		);
	});
});
