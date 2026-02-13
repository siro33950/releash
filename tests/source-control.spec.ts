import { expect, test } from "@playwright/test";
import {
	buildMockConfig,
	mixedChanges,
	stagedChanges,
	unstagedChanges,
} from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { getInvokeHistory, trackInvocations, waitForApp } from "./helpers/utils";

/**
 * WorktreeView の SourceControlPanel テスト。
 * worktree が1つだけの場合、App.tsx が自動で WorktreeView を開く。
 */
function worktreeViewConfig(overrides: Record<string, unknown> = {}) {
	return buildMockConfig({
		list_worktrees: [
			{
				name: "repo",
				path: "/test/repo",
				branch: "feat/test",
				is_main: true,
				is_locked: false,
				dirty_count: 0,
				base_branch: null,
			},
		],
		get_current_branch: "feat/test",
		...overrides,
	});
}

test.describe("Source Control", () => {
	test("変更ファイル一覧が表示される", async ({ page }) => {
		const config = worktreeViewConfig({
			get_git_status: unstagedChanges,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// SourceControlPanel は activeView="git" がデフォルト
		// Unstaged Files セクションのファイル名が表示される
		await expect(page.getByText("App.tsx").first()).toBeVisible();
		await expect(page.getByText("README.md").first()).toBeVisible();
	});

	test("Staged/Unstagedファイルがそれぞれ表示される", async ({ page }) => {
		const config = worktreeViewConfig({
			get_git_status: mixedChanges,
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// ヘッダーに合計変更数が表示される
		await expect(page.getByText("3 file changes")).toBeVisible();

		// Unstaged Files セクション
		await expect(page.getByText("Unstaged Files (2)")).toBeVisible();
		// Staged Files セクション
		await expect(page.getByText("Staged Files (1)")).toBeVisible();
	});

	test("Stage操作: git_stage が invoke される", async ({ page }) => {
		const config = worktreeViewConfig({
			get_git_status: unstagedChanges,
			git_stage: null,
		});
		await setupTauriMock(page, config);
		await trackInvocations(page);
		await waitForApp(page);

		// Unstaged Files セクションの "Stage All Changes" ボタン（小さいアイコンボタン）
		const stageAllBtn = page.getByRole("button", {
			name: "Stage All Changes",
			exact: true,
		});
		await stageAllBtn.click({ force: true });

		// git_stage は非同期で呼ばれるので待機
		await page.waitForFunction(
			() =>
				// @ts-expect-error - テスト用グローバル
				(window.__INVOKE_HISTORY__ ?? []).some(
					(h: { cmd: string }) => h.cmd === "git_stage",
				),
			null,
			{ timeout: 5000 },
		);

		const history = await getInvokeHistory(page);
		const stageCall = history.find((h) => h.cmd === "git_stage");
		expect(stageCall).toBeTruthy();
	});

	test("Unstage操作: git_unstage が invoke される", async ({ page }) => {
		const config = worktreeViewConfig({
			get_git_status: stagedChanges,
			git_unstage: null,
		});
		await setupTauriMock(page, config);
		await trackInvocations(page);
		await waitForApp(page);

		// Staged Files セクションの "Unstage All Changes" ボタン
		const unstageAllBtn = page.getByTitle("Unstage All Changes");
		await unstageAllBtn.click();

		await page.waitForFunction(
			() =>
				// @ts-expect-error - テスト用グローバル
				(window.__INVOKE_HISTORY__ ?? []).some(
					(h: { cmd: string }) => h.cmd === "git_unstage",
				),
			null,
			{ timeout: 5000 },
		);

		const history = await getInvokeHistory(page);
		const unstageCall = history.find((h) => h.cmd === "git_unstage");
		expect(unstageCall).toBeTruthy();
	});

	test("コミットメッセージ入力 → Commit", async ({ page }) => {
		const config = worktreeViewConfig({
			get_git_status: stagedChanges,
			git_commit: "abc1234",
		});
		await setupTauriMock(page, config);
		await trackInvocations(page);
		await waitForApp(page);

		// コミットメッセージ入力
		const summaryInput = page.getByPlaceholder("Commit summary");
		await summaryInput.fill("test: add integration tests");

		// Commit ボタンをクリック
		const commitBtn = page.getByRole("button", { name: "Commit" });
		await commitBtn.click();

		await page.waitForFunction(
			() =>
				// @ts-expect-error - テスト用グローバル
				(window.__INVOKE_HISTORY__ ?? []).some(
					(h: { cmd: string }) => h.cmd === "git_commit",
				),
			null,
			{ timeout: 5000 },
		);

		const history = await getInvokeHistory(page);
		const commitCall = history.find((h) => h.cmd === "git_commit");
		expect(commitCall).toBeTruthy();
	});

	test("Push操作: git_push が invoke される", async ({ page }) => {
		const config = worktreeViewConfig({
			get_git_status: [],
			git_push: "Everything up-to-date",
		});
		await setupTauriMock(page, config);
		await trackInvocations(page);
		await waitForApp(page);

		// Push ボタンをクリック
		const pushBtn = page.getByRole("button", { name: "Push" });
		await pushBtn.click();

		await page.waitForFunction(
			() =>
				// @ts-expect-error - テスト用グローバル
				(window.__INVOKE_HISTORY__ ?? []).some(
					(h: { cmd: string }) => h.cmd === "git_push",
				),
			null,
			{ timeout: 5000 },
		);

		const history = await getInvokeHistory(page);
		const pushCall = history.find((h) => h.cmd === "git_push");
		expect(pushCall).toBeTruthy();
	});
});
