import { expect, test } from "@playwright/test";
import { buildMockConfig } from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";
import { waitForApp } from "./helpers/utils";
import backendXtermFixture from "./fixtures/terminal-surface-checkpoint-v1.json" with {
	type: "json",
};

test("Terminal Surface接続はbackend resizeより先にsnapshotを投影する", async ({
	page,
}) => {
	await setupTauriMock(
		page,
		buildMockConfig({
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
		}),
	);
	await waitForApp(page);

	await expect
		.poll(() =>
			page.evaluate(() =>
				window.__TAURI_INTERNALS__?.invocations.some(
					(invocation) => invocation.cmd === "resize_terminal_surface",
				),
			),
		)
			.toBe(true);
});

test("Terminal Surfaceのproduction wireをreload後もsnapshotとlive outputとして投影する", async ({
	page,
}) => {
	const snapshot = {
		session_key: "workspace:10:/test/repo",
		terminal_surface: {
			replay: "\u001bc\u001b[H\u001b[2Jcheckpoint-before-reload",
			sequence: 40,
			cols: 80,
			rows: 24,
		},
		is_exited: false,
		exit_code: null,
	};
	await setupTauriMock(
		page,
		buildMockConfig({
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
			get_or_spawn_terminal_surface: {
				session_key: snapshot.session_key,
				restored_from_checkpoint: false,
				is_new: false,
				is_exited: false,
				exit_code: null,
			},
			get_terminal_surface: snapshot,
			attach_terminal_surface: {
				__mockTerminalAttachment: true,
				messages: [
					{ type: "snapshot", surface: snapshot },
					{
						type: "output",
						session_key: snapshot.session_key,
						data: "\r\nlive-after-attach",
						sequence: 41,
					},
				],
			},
		}),
	);
	await waitForApp(page);

	await expect(page.locator(".xterm-rows")).toContainText(
		"checkpoint-before-reload",
	);
	await expect(page.locator(".xterm-rows")).toContainText("live-after-attach");

	await page.reload();
	await waitForApp(page);
	await expect(page.locator(".xterm-rows")).toContainText(
		"checkpoint-before-reload",
	);
	await expect(page.locator(".xterm-rows")).toContainText("live-after-attach");
});

test("backend AVT生成checkpointを実xtermへalternate screen・属性・wide文字・cursor込みで投影する", async ({
	page,
}) => {
	const snapshot = {
		session_key: "workspace:10:/test/repo",
		terminal_surface: backendXtermFixture.checkpoint,
		is_exited: false,
		exit_code: null,
	};
	await setupTauriMock(
		page,
		buildMockConfig({
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
			get_or_spawn_terminal_surface: {
				session_key: snapshot.session_key,
				restored_from_checkpoint: true,
				is_new: true,
				is_exited: false,
				exit_code: null,
			},
			get_terminal_surface: snapshot,
			attach_terminal_surface: {
				__mockTerminalAttachment: true,
				messages: [{ type: "snapshot", surface: snapshot }],
			},
		}),
	);
	await waitForApp(page);

	const rows = page.locator(".xterm-rows");
	await expect(rows).toContainText("ALT-SCREEN");
	await expect(rows).not.toContainText("PRIMARY-GREEN");
	await expect(rows).toContainText("日本語🙂");
	await expect(rows).toContainText("RED-BOLD");

	const redBold = rows.locator("span").filter({ hasText: "RED-BOLD" });
	await expect(redBold).toHaveCSS("font-weight", "700");
	await expect(redBold).toHaveCSS("color", "rgb(239, 41, 41)");
	const wideJapanese = rows.locator("span").filter({ hasText: "日本語" });
	const wideEmoji = rows.locator("span").filter({ hasText: "🙂" });
	const narrow = rows.locator("span").filter({ hasText: "ABCD" });
	const [wideJapaneseBox, wideEmojiBox, narrowBox] = await Promise.all([
		wideJapanese.boundingBox(),
		wideEmoji.boundingBox(),
		narrow.boundingBox(),
	]);
	expect(wideJapaneseBox).not.toBeNull();
	expect(wideEmojiBox).not.toBeNull();
	expect(narrowBox).not.toBeNull();
	expect(wideJapaneseBox!.width + wideEmojiBox!.width).toBeGreaterThan(
		narrowBox!.width * 1.7,
	);

	await expect(page.locator(".xterm-rows > div").nth(4).locator(".xterm-cursor"))
		.toBeVisible();
});
