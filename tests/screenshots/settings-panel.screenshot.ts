import { expect, test } from "@playwright/test";
import {
	setupWorktreeView,
	switchToView,
	xtermMask,
} from "../helpers/screenshot-utils";

test.describe("Settings Panel", () => {
	async function openSettings(
		page: Parameters<typeof setupWorktreeView>[0],
		overrides: Record<string, unknown> = {},
	) {
		await setupWorktreeView(page, overrides);
		await switchToView(page, "Settings");
		await page.waitForTimeout(300);
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
		// Editor セクションまでスクロール
		const editorHeading = page.getByText("Editor", { exact: true });
		await editorHeading.scrollIntoViewIfNeeded();
		await page.waitForTimeout(200);
		await expect(page).toHaveScreenshot(
			"worktree-settings-editor.png",
			{ mask: xtermMask(page) },
		);
	});

	test("agent section with claude selected", async ({ page }) => {
		await openSettings(page);
		const agentHeading = page.getByText("Agent", { exact: true }).first();
		await agentHeading.scrollIntoViewIfNeeded();
		await page.waitForTimeout(200);
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
		const agentHeading = page.getByText("Agent", { exact: true }).first();
		await agentHeading.scrollIntoViewIfNeeded();
		await page.waitForTimeout(200);
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
		const agentHeading = page.getByText("Agent", { exact: true }).first();
		await agentHeading.scrollIntoViewIfNeeded();
		await page.waitForTimeout(200);
		await expect(page).toHaveScreenshot(
			"worktree-settings-agent-none.png",
			{ mask: xtermMask(page) },
		);
	});

	test("remote section", async ({ page }) => {
		await openSettings(page);
		const remoteHeading = page.getByText("Remote", { exact: true }).first();
		await remoteHeading.scrollIntoViewIfNeeded();
		await page.waitForTimeout(200);
		await expect(page).toHaveScreenshot(
			"worktree-settings-remote.png",
			{ mask: xtermMask(page) },
		);
	});

	test("notifications section", async ({ page }) => {
		await openSettings(page);
		const notifHeading = page.getByText("Notifications", { exact: true });
		await notifHeading.scrollIntoViewIfNeeded();
		await page.waitForTimeout(200);
		await expect(page).toHaveScreenshot(
			"worktree-settings-notifications.png",
			{ mask: xtermMask(page) },
		);
	});

	test("privacy and updates section", async ({ page }) => {
		await openSettings(page);
		const privacyHeading = page.getByText("Privacy & Updates");
		await privacyHeading.scrollIntoViewIfNeeded();
		await page.waitForTimeout(200);
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
		const saveBtn = page.getByRole("button", { name: "Save" });
		await saveBtn.scrollIntoViewIfNeeded();
		await page.waitForTimeout(200);
		await expect(page).toHaveScreenshot(
			"worktree-settings-save-enabled.png",
			{ mask: xtermMask(page) },
		);
	});
});
