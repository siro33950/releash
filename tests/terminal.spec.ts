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
			"worktrees": [
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
				window.__RELEASH_BACKEND__?.invocations.some(
					(invocation) => invocation.cmd === "resize_terminal_surface",
				),
			),
		)
		.toBe(true);
});

test("stream再接続前後の複数入力は新attachmentの連番で届き切断中は失敗表示される", async ({ page }) => {
	const mock = await setupTauriMock(page, buildMockConfig({
		worktrees: [{
			name: "repo", path: "/test/repo", branch: "feat/test", is_main: true,
			is_locked: false, dirty_count: 0, base_branch: null,
		}],
	}));
	await waitForApp(page);
	const terminal = page.locator(".xterm-helper-textarea").first();
	await terminal.focus();
	await page.keyboard.type("ab");
	const accounted = (data: string) =>
		mock.terminalDelivered.some((item) => item.data === data) ||
		mock.terminalFailures.some((item) => item.data === data);
	await expect.poll(() => mock.clientRequests.filter(({ command }) => command === "write_terminal_surface").length).toBe(2);
	await expect.poll(() => ["a", "b"].every(accounted)).toBe(true);
	const before = mock.clientRequests.filter(({ command }) => command === "write_terminal_surface");
	expect(before.map(({ args }) => args.data)).toEqual(["a", "b"]);
	expect(mock.terminalDelivered.map(({ data }) => data)).toEqual(["a", "b"].filter((data) => mock.terminalDelivered.some((item) => item.data === data)));
	if (mock.terminalFailures.some(({ data }) => data === "a" || data === "b")) {
		await expect(page.getByText(/Terminal input may have been executed/)).toBeVisible();
	} else {
		expect(mock.terminalDelivered.map(({ sequence }) => sequence)).toEqual([0, 1]);
	}
	const deliveredBefore = [...mock.terminalDelivered];
	const failuresBefore = [...mock.terminalFailures];
	mock.disconnectStateStreams();
	await expect.poll(() => page.evaluate(async () => (await import("/src/lib/client.ts")).getConnectionState())).toBe("TRANSIENT_FAILURE");
	await page.keyboard.type("c");
	await expect(page.getByText("Daemon connection is TRANSIENT_FAILURE")).toBeVisible();
	expect(mock.clientRequests.filter(({ command }) => command === "write_terminal_surface")).toHaveLength(2);
	expect(mock.terminalDelivered).toEqual(deliveredBefore);
	expect(mock.terminalFailures).toEqual(failuresBefore);
	await expect.poll(() => page.evaluate(async () => (await import("/src/lib/client.ts")).getConnectionState())).toBe("READY");
	await expect.poll(() => mock.clientRequests.filter(({ command }) => command === "start_state_subscription").length).toBeGreaterThan(1);
	await page.keyboard.type("de");
	await expect.poll(() => mock.clientRequests.filter(({ command }) => command === "write_terminal_surface").length).toBe(4);
	await expect.poll(() => ["d", "e"].every(accounted)).toBe(true);
	const writes = mock.clientRequests.filter(({ command }) => command === "write_terminal_surface");
	expect(writes.map(({ args }) => args.data)).toEqual(["a", "b", "d", "e"]);
	expect(writes[2].args.attachmentId).not.toBe(writes[1].args.attachmentId);
	const after = mock.terminalDelivered.filter(({ data }) => data === "d" || data === "e");
	expect(after.map(({ data }) => data)).toEqual(["d", "e"].filter((data) => after.some((item) => item.data === data)));
	for (const item of after) {
		expect(item.attachmentId).toBe(writes[item.data === "d" ? 2 : 3].args.attachmentId);
	}
	if (mock.terminalFailures.some(({ data }) => data === "d" || data === "e")) {
		await expect(page.getByText(/Terminal input may have been executed/)).toBeVisible();
	} else {
		expect(after.map(({ sequence }) => sequence)).toEqual([0, 1]);
		expect(writes[3].args.attachmentId).toBe(writes[2].args.attachmentId);
	}
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
			"worktrees": [
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
			},
			terminal_snapshot: {
				session_key: snapshot.session_key,
				is_exited: snapshot.is_exited,
				exit_code: snapshot.exit_code,
			},
			start_state_subscription: {
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
			"worktrees": [
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
			},
			terminal_snapshot: {
				session_key: snapshot.session_key,
				is_exited: snapshot.is_exited,
				exit_code: snapshot.exit_code,
			},
			start_state_subscription: {
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

	await expect(
		page.locator(".xterm-rows > div").nth(4).locator(".xterm-cursor"),
	).toBeVisible();
});
