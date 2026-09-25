import { expect, test } from "@playwright/test";
import { buildMockConfig } from "./helpers/fixtures";
import { setupTauriMock } from "./helpers/tauri-mock";

test("20terminalとpushを保持しても入力・処理済み量通知・状態取得がHTTP/1.1で継続する", async ({
	page,
}) => {
	await setupTauriMock(page, buildMockConfig({
			get_external_editor: "/current",
			start_state_subscription: { __mockTerminalAttachment: true },
			write_terminal_surface: null,
			report_terminal_processed: null,
			stop_state_subscription: null,
	}));
	await page.goto("/tests/helpers/client-streams.html");
	const subscriptions: string[] = [];
	page.on("request", (request) => {
		if (/\/(?:SubscribePush|OpenStateStream)$/.test(request.url()))
			subscriptions.push(request.url());
	});
	const result = await page.evaluate(async () => {
		const {
			subscribeTerminalState,
			reportTerminalProcessed,
			getClient,
			listenClient,
			invokeClient,
		} = await import("/src/lib/client.ts");
		const events = new Map<string, string[]>();
		let pushed = false;
		const stopPush = await listenClient("review-comments-changed", () => {
			pushed = true;
		});
		const releases: Array<() => Promise<void>> = [];
		for (let index = 0; index < 20; index++) {
			const id = `pane-${index}`;
			events.set(id, []);
			releases.push(
				await subscribeTerminalState(
					{
						owner: { kind: "workspace", workspacePath: `/repo-${index}` },
						attachmentId: id,
					},
					(item) => {
						if (item.type === "output") events.get(id)!.push(item.data);
					},
					() => {
						throw new Error("Unexpected terminal closure");
					},
				),
			);
		}
		const client = await getClient();
		await Promise.all(
			[...events.keys()].map(async (attachmentId, index) => {
				await client.writeTerminalSurface({
					owner: {
						variant: {
							case: "workspace",
							value: { workspacePath: `/repo-${index}` },
						},
					},
					attachmentId,
					sequence: 0n,
					data: `input-${index}`,
				});
				await window.__releashTerminalEvent(attachmentId, {
					type: "output",
					session_key: attachmentId,
					data: `output-${index}`,
					sequence: 42,
				});
				await reportTerminalProcessed({kind: "workspace", workspacePath: `/repo-${index}`}, 5000);
				await window.__releashTerminalEvent(attachmentId, {
					type: "output",
					session_key: attachmentId,
					data: `resumed-${index}`,
					sequence: 43,
				});
			}),
		);
		const paths = await invokeClient("get_external_editor");
		await window.__releashPush("review-comments-changed", "/updated");
		const deadline = Date.now() + 3000;
		while (
			(!pushed || [...events.values()].some((output) => output.length < 2)) &&
			Date.now() < deadline
		)
			await new Promise((resolve) => setTimeout(resolve, 10));
		await Promise.all(releases.map((release) => release()));
		stopPush();
		return {
			paths,
			pushed,
			outputs: [...events.values()],
			inputs: window
				.__RELEASH_BACKEND__!.invocations.filter(
					(item) => item.cmd === "write_terminal_surface",
				)
				.map((item) => item.args),
			acks: window
				.__RELEASH_BACKEND__!.invocations.filter(
					(item) => item.cmd === "report_terminal_processed",
				)
				.map((item) => item.args),
		};
	});
	expect(
		subscriptions.filter((url) => url.endsWith("/SubscribePush")),
	).toHaveLength(1);
	expect(
		subscriptions.filter((url) => url.endsWith("/OpenStateStream")),
	).toHaveLength(1);
	expect(result.paths).toEqual("/current");
	expect(result.pushed).toBe(true);
	expect(result.outputs).toEqual(
		Array.from({ length: 20 }, (_, index) => [
			`output-${index}`,
			`resumed-${index}`,
		]),
	);
	expect(result.inputs).toHaveLength(20);
	expect(result.acks).toHaveLength(20);
	for (let index = 0; index < 20; index++) {
		expect(result.inputs).toContainEqual(
			expect.objectContaining({
				attachmentId: `pane-${index}`,
				data: `input-${index}`,
			}),
		);
		expect(result.acks).toContainEqual({
			clientId: expect.any(String), args: [`/repo-${index}`], units: 5000,
		});
	}
});


test("購読fixtureは初回と更新のpayloadを単発commandなしで配信する", async ({page}) => {
    const fixture = await setupTauriMock(page, buildMockConfig({"current-branch": "before"}));
    await page.goto("/tests/helpers/client-streams.html");
    await page.evaluate(async () => {
        const {subscribeState, firstState} = await import("/src/lib/client.ts");
        await Promise.all([
            firstState("workspaces"),
            firstState("providers"),
            firstState({kind: "branch-status", args: ["/repo"]}),
            firstState({kind: "worktrees", args: ["/repo"]}),
        ]);
        subscribeState({kind: "current-branch", args: ["/repo"]}, value => {
            document.body.textContent = value;
        });
    });
    await expect(page.locator("body")).toHaveText("before");
    await page.evaluate(() => window.__RELEASH_BACKEND__!.setState("current-branch", "after"));
    await fixture.refreshStates();
    await expect(page.locator("body")).toHaveText("after");
    expect(fixture.clientRequests).toEqual([]);
    expect(await page.evaluate(() => window.__RELEASH_BACKEND__!.invocations.map(({cmd}) => cmd))).toEqual(["get_client_endpoint", "validate_daemon_connection"]);
});
