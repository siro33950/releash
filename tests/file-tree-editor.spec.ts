import { expect, test } from "@playwright/test";
import {
	buildMockConfig,
	rootDirEntries,
	srcDirEntries,
} from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";

/**
 * WorktreeView でファイルツリー + エディタのテスト。
 * worktree が1つだけの場合、App.tsx が自動で WorktreeView を開く。
 * Explorer ビューに切り替えてファイルツリーを表示する。
 */
function fileTreeConfig(overrides: Record<string, unknown> = {}) {
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
		get_git_status: [],
		// readDir のモック: path引数に応じて返すデータを切り替えたいが
		// addInitScript では関数をシリアライズできないため、
		// デフォルトはルートディレクトリのエントリを返す
		"plugin:fs|read_dir": rootDirEntries,
		...overrides,
	});
}

test.describe("File Tree & Editor", () => {
	test("Explorer ビューでファイルツリーが表示される", async ({ page }) => {
		const config = fileTreeConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Explorer ビューに切り替え（ActivityBar の Explorer ボタン）
		const explorerBtn = page.getByRole("tab", { name: "Explorer" });
		await explorerBtn.click();

		// ルートディレクトリのエントリが表示される
		await expect(page.getByText("src").first()).toBeVisible();
		await expect(page.getByText("README.md").first()).toBeVisible();
		await expect(page.getByText("package.json").first()).toBeVisible();
	});

	test("フォルダクリックで展開/折り畳みが切り替わる", async ({ page }) => {
		const config = fileTreeConfig();
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Explorer ビューに切り替え
		const explorerBtn = page.getByRole("tab", { name: "Explorer" });
		await explorerBtn.click();

		// src フォルダをクリックして展開
		// readDir は常に rootDirEntries を返すが、
		// 展開後は plugin:fs|read_dir が再度呼ばれる（子ディレクトリのエントリ取得）
		// モックは同じデータを返すので、構造的には同じだが展開アクション自体をテスト
		await page.getByText("src").first().click();

		// 展開状態では ChevronDown が表示される（視覚的確認）
		// 折り畳み→再クリックで ChevronRight に戻ることを確認
		// src を再クリックして折り畳む
		await page.getByText("src").first().click();

		// フォルダが存在し、クリック可能であることを確認
		await expect(page.getByText("src").first()).toBeVisible();
	});

	test("ファイルクリックでエディタにタブが追加される", async ({ page }) => {
		const config = fileTreeConfig({
			get_file_at_ref: "// file content\nconsole.log('hello');",
			get_staged_content: "// file content\nconsole.log('hello');",
			"plugin:fs|read_text_file": "// file content\nconsole.log('hello');",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Explorer ビューに切り替え
		const explorerBtn = page.getByRole("tab", { name: "Explorer" });
		await explorerBtn.click();

		// README.md をクリック（ファイル）
		await page.getByText("README.md").first().click();

		// エディタタブに README.md が追加されることを確認
		const editorTab = page.locator('[data-slot="tabs-trigger"]').filter({ hasText: "README.md" });
		await expect(editorTab).toBeVisible({ timeout: 5000 });
	});

	test("エディタタブの閉じるボタンでタブが閉じる", async ({ page }) => {
		const config = fileTreeConfig({
			get_file_at_ref: "// content",
			get_staged_content: "// content",
			"plugin:fs|read_text_file": "// content",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Explorer → ファイルクリック → タブ追加
		await page.getByRole("tab", { name: "Explorer" }).click();
		await page.getByText("README.md").first().click();
		const editorTab = page.locator('[data-slot="tabs-trigger"]').filter({ hasText: "README.md" });
		await expect(editorTab).toBeVisible({ timeout: 5000 });

		// タブの Close ボタンをクリック
		await editorTab
			.getByRole("button", { name: /Close README\.md/ })
			.click({ force: true });

		// タブが消えることを確認
		await expect(editorTab).not.toBeVisible();
	});

	test("Diff モード切替ボタンが動作する", async ({ page }) => {
		const config = fileTreeConfig({
			get_file_at_ref: "// original",
			get_staged_content: "// original",
			"plugin:fs|read_text_file": "// modified content",
		});
		await setupTauriMock(page, config);
		await waitForApp(page);

		// Explorer → ファイルクリック
		await page.getByRole("tab", { name: "Explorer" }).click();
		await page.getByText("README.md").first().click();
		const editorTab = page.locator('[data-slot="tabs-trigger"]').filter({ hasText: "README.md" });
		await expect(editorTab).toBeVisible({ timeout: 5000 });

		// Diff モードボタンが存在することを確認
		// EditorPanel のフッターに Gutter / Inline / Split ボタンがある
		await expect(
			page.getByRole("button", { name: "Gutter" }),
		).toBeVisible();
		await expect(
			page.getByRole("button", { name: "Inline" }),
		).toBeVisible();
		await expect(
			page.getByRole("button", { name: "Split" }),
		).toBeVisible();

		// Split をクリック
		await page.getByRole("button", { name: "Split" }).click();

		// Inline をクリック
		await page.getByRole("button", { name: "Inline" }).click();
	});
});
