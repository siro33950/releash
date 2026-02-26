import { expect, test } from "@playwright/test";
import { branchList } from "../helpers/fixtures";
import {
	collapsePanels,
	setupWorktreeView,
	switchToView,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("Settings Modal", () => {
	async function openSettings(
		page: Parameters<typeof setupWorktreeView>[0],
		overrides: Record<string, unknown> = {},
	) {
		await setupWorktreeView(page, overrides);
		await collapsePanels(page);
		await switchToView(page, "Settings");
		await page.waitForTimeout(300);
	}

	async function switchSettingsTab(
		page: Parameters<typeof setupWorktreeView>[0],
		tabName: string,
	) {
		const tab = page.locator("nav").getByText(tabName, { exact: true });
		await tab.click();
		await expect(tab).toHaveClass(/bg-accent/);
	}

	test("appearance section (default)", async ({ page }) => {
		await openSettings(page);
		await expect(page).toHaveScreenshot(
			"worktree-settings-appearance.png",
			{ mask: xtermMask(page) },
		);
	});

	test("editor section", async ({ page }) => {
		await openSettings(page);
		await switchSettingsTab(page, "Editor");
		await expect(page).toHaveScreenshot(
			"worktree-settings-editor.png",
			{ mask: xtermMask(page) },
		);
	});

	test("agent section with claude selected", async ({ page }) => {
		await openSettings(page);
		await switchSettingsTab(page, "Agent");
		// Agent ドロップダウンで claude を選択
		const agentSelect = page.locator("select").filter({ hasText: "None" });
		if (await agentSelect.isVisible()) {
			await agentSelect.selectOption("claude");
			await page.waitForTimeout(500);
		}
		await expect(page).toHaveScreenshot(
			"worktree-settings-agent-claude.png",
			{ mask: xtermMask(page) },
		);
	});

	test("agent section with custom selected", async ({ page }) => {
		await openSettings(page);
		await switchSettingsTab(page, "Agent");
		const agentSelect = page.locator("select").filter({ hasText: "None" });
		if (await agentSelect.isVisible()) {
			await agentSelect.selectOption("custom");
			await page.waitForTimeout(300);
		}
		await expect(page).toHaveScreenshot(
			"worktree-settings-agent-custom.png",
			{ mask: xtermMask(page) },
		);
	});

	test("agent section with none selected", async ({ page }) => {
		await openSettings(page);
		await switchSettingsTab(page, "Agent");
		await expect(page).toHaveScreenshot(
			"worktree-settings-agent-none.png",
			{ mask: xtermMask(page) },
		);
	});

	test("remote section", async ({ page }) => {
		await openSettings(page);
		await switchSettingsTab(page, "Remote");
		await expect(page).toHaveScreenshot(
			"worktree-settings-remote.png",
			{ mask: xtermMask(page) },
		);
	});

	test("notifications section", async ({ page }) => {
		await openSettings(page);
		await switchSettingsTab(page, "Notifications");
		await expect(page).toHaveScreenshot(
			"worktree-settings-notifications.png",
			{ mask: xtermMask(page) },
		);
	});

	test("privacy and updates section", async ({ page }) => {
		await openSettings(page);
		await switchSettingsTab(page, "Privacy & Updates");
		await expect(page).toHaveScreenshot(
			"worktree-settings-privacy.png",
			{ mask: xtermMask(page) },
		);
	});

	test("save button enabled after change", async ({ page }) => {
		await openSettings(page);
		// テーマを変更して Save ボタンが有効になることを確認
		const lightBtn = page.getByText("Light").first();
		if (await lightBtn.isVisible()) {
			await lightBtn.click();
			await page.waitForTimeout(200);
		}
		await expect(page).toHaveScreenshot(
			"worktree-settings-save-enabled.png",
			{ mask: xtermMask(page) },
		);
	});

	// --- Background Section ---
	test("background section (default)", async ({ page }) => {
		await openSettings(page);
		await switchSettingsTab(page, "Background");
		await expect(page).toHaveScreenshot(
			"worktree-settings-background.png",
			{ mask: xtermMask(page) },
		);
	});

	test("background section with auto-launch enabled", async ({ page }) => {
		await openSettings(page, {
			"plugin:autostart|is_enabled": true,
		});
		await switchSettingsTab(page, "Background");
		await expect(page).toHaveScreenshot(
			"worktree-settings-background-autolaunch.png",
			{ mask: xtermMask(page) },
		);
	});

	// --- Repositories Section ---
	test("repositories section (default)", async ({ page }) => {
		await openSettings(page, {
			list_branches: branchList,
			get_releash_base: "main",
		});
		await switchSettingsTab(page, "Repositories");
		await page.waitForTimeout(500);
		await expect(page).toHaveScreenshot(
			"worktree-settings-repositories.png",
			{ mask: xtermMask(page) },
		);
	});

	test("repositories section (empty)", async ({ page }) => {
		await openSettings(page, {
			list_worktrees: [],
		});
		await switchSettingsTab(page, "Repositories");
		await expect(page).toHaveScreenshot(
			"worktree-settings-repositories-empty.png",
			{ mask: xtermMask(page) },
		);
	});
});
