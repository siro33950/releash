import { expect, test } from "@playwright/test";
import { searchResults } from "../helpers/fixtures";
import {
	setupWorktreeView,
	switchToView,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("Search Panel", () => {
	test("empty state", async ({ page }) => {
		await setupWorktreeView(page);
		await switchToView(page, "Search");
		await page.waitForTimeout(300);
		await expect(page).toHaveScreenshot(
			"worktree-search-empty.png",
			{ mask: xtermMask(page) },
		);
	});

	test("with results", async ({ page }) => {
		await setupWorktreeView(page, {
			search_files: searchResults,
		});
		await switchToView(page, "Search");
		await page.waitForTimeout(300);
		const searchInput = page.getByPlaceholder("Search");
		await searchInput.fill("useState");
		// デバウンス待ち
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-search-results.png",
			{ mask: xtermMask(page) },
		);
	});

	test("no results", async ({ page }) => {
		await setupWorktreeView(page, {
			search_files: { matches: [], total_matches: 0, truncated: false },
		});
		await switchToView(page, "Search");
		await page.waitForTimeout(300);
		const searchInput = page.getByPlaceholder("Search");
		await searchInput.fill("nonexistent_term_xyz");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-search-no-results.png",
			{ mask: xtermMask(page) },
		);
	});

	test("case sensitive toggle active", async ({ page }) => {
		await setupWorktreeView(page, {
			search_files: searchResults,
		});
		await switchToView(page, "Search");
		await page.waitForTimeout(300);
		// Match Case ボタンをクリック
		const caseSensitiveBtn = page.getByTitle("Match Case");
		await caseSensitiveBtn.click();
		const searchInput = page.getByPlaceholder("Search");
		await searchInput.fill("useState");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-search-case-sensitive.png",
			{ mask: xtermMask(page) },
		);
	});

	test("regex toggle active", async ({ page }) => {
		await setupWorktreeView(page, {
			search_files: searchResults,
		});
		await switchToView(page, "Search");
		await page.waitForTimeout(300);
		// Regex ボタンをクリック
		const regexBtn = page.getByTitle("Use Regular Expression");
		await regexBtn.click();
		const searchInput = page.getByPlaceholder("Search");
		await searchInput.fill("use.*State");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-search-regex.png",
			{ mask: xtermMask(page) },
		);
	});

	test("both case sensitive and regex active", async ({ page }) => {
		await setupWorktreeView(page, {
			search_files: searchResults,
		});
		await switchToView(page, "Search");
		await page.waitForTimeout(300);
		const caseSensitiveBtn = page.getByTitle("Match Case");
		await caseSensitiveBtn.click();
		const regexBtn = page.getByTitle("Use Regular Expression");
		await regexBtn.click();
		const searchInput = page.getByPlaceholder("Search");
		await searchInput.fill("use[A-Z]\\w+");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-search-case-regex.png",
			{ mask: xtermMask(page) },
		);
	});
});
