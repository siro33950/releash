import type { Locator, Page } from "@playwright/test";
import type { WorktreeEntry } from "./fixtures";
import { buildMockConfig, type MockConfig } from "./fixtures";
import { setupTauriMock } from "./tauri-mock";
import { waitForApp } from "./utils";

const defaultWorktree: WorktreeEntry = {
	name: "repo",
	path: "/test/repo",
	branch: "feat/test",
	is_main: true,
	is_locked: false,
	dirty_count: 0,
	base_branch: null,
};

/**
 * WorktreeView を表示する（worktree 1個で自動遷移）。
 * overrides で IPC レスポンスを差し替え可能。
 */
export async function setupWorktreeView(
	page: Page,
	overrides: Record<string, unknown> = {},
): Promise<MockConfig> {
	const config = buildMockConfig({
		list_worktrees: [defaultWorktree],
		get_current_branch: "feat/test",
		...overrides,
	});
	await setupTauriMock(page, config);
	await waitForApp(page);
	return config;
}

/**
 * WorkspaceManagerScreen を表示する（worktree 0個で Kanban 表示）。
 */
export async function setupWorkspaceManager(
	page: Page,
	overrides: Record<string, unknown> = {},
): Promise<MockConfig> {
	const config = buildMockConfig({
		list_worktrees: [],
		...overrides,
	});
	await setupTauriMock(page, config);
	await waitForApp(page);
	return config;
}

/**
 * ActivityBar でビュー切り替え。
 */
export async function switchToView(
	page: Page,
	viewName: string,
): Promise<void> {
	const btn = page.getByRole("button", { name: viewName, exact: true });
	await btn.click();
	await page.waitForTimeout(300);
}

/**
 * Monaco Editor 領域のマスク用ロケーター。
 * スクリーンショット比較時に非決定的なエディタ内容をマスクする。
 * UI_REVIEW=1 の場合はマスクを無効化する。
 */
export function monacoMask(page: Page): Locator[] {
	if (process.env.UI_REVIEW === "1") return [];
	return [page.locator(".monaco-editor")];
}

/**
 * xterm ターミナル領域のマスク用ロケーター。
 * UI_REVIEW=1 の場合はマスクを無効化する。
 */
export function xtermMask(page: Page): Locator[] {
	if (process.env.UI_REVIEW === "1") return [];
	return [page.locator(".xterm")];
}
