import { expect, test } from "@playwright/test";
import { setupTauriMock } from "./helpers/tauri-mock";

test("最大16terminalとpushを保持しても入力・ack・状態取得がHTTP/1.1で継続する", async ({
	page,
}) => {
	await setupTauriMock(page, {
		responses: {
			get_repo_paths: ["/current"],
			attach_terminal_surface: { __mockTerminalAttachment: true },
			write_terminal_surface: null,
			ack_terminal_surface_output: null,
			detach_terminal_surface: null,
		},
	});
	await page.goto("/tests/helpers/client-streams.html");
	const subscriptions: string[] = [];
	page.on("request", (request) => {
		if (/\/Subscribe(?:Push|TerminalSurfaces)$/.test(request.url()))
			subscriptions.push(request.url());
	});
	const result = await page.evaluate(async () => {
		const {
			attachClientStream,
			acknowledgeClientStream,
			getClient,
			listenClient,
			invokeClient,
		} = await import("/src/lib/client.ts");
		const events = new Map<string, string[]>();
		let pushed = false;
		const stopPush = await listenClient("repo-paths-changed", () => {
			pushed = true;
		});
		const releases: Array<() => Promise<void>> = [];
		for (let index = 0; index < 16; index++) {
			const id = `pane-${index}`;
			events.set(id, []);
			releases.push(
				await attachClientStream(
					{
						owner: { kind: "workspace", workspacePath: `/repo-${index}` },
						attachmentId: id,
						recovery: false,
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
				await acknowledgeClientStream(attachmentId, 42);
				await window.__releashTerminalEvent(attachmentId, {
					type: "output",
					session_key: attachmentId,
					data: `resumed-${index}`,
					sequence: 43,
				});
			}),
		);
		const paths = await invokeClient("get_repo_paths");
		await window.__releashPush("repo-paths-changed", ["/updated"]);
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
					(item) => item.cmd === "ack_terminal_surface_output",
				)
				.map((item) => item.args),
		};
	});
	expect(
		subscriptions.filter((url) => url.endsWith("/SubscribePush")),
	).toHaveLength(1);
	expect(
		subscriptions.filter((url) => url.endsWith("/SubscribeTerminalSurfaces")),
	).toHaveLength(1);
	expect(result.paths).toEqual(["/current"]);
	expect(result.pushed).toBe(true);
	expect(result.outputs).toEqual(
		Array.from({ length: 16 }, (_, index) => [
			`output-${index}`,
			`resumed-${index}`,
		]),
	);
	expect(result.inputs).toHaveLength(16);
	expect(result.acks).toHaveLength(16);
	for (let index = 0; index < 16; index++) {
		expect(result.inputs).toContainEqual(
			expect.objectContaining({
				attachmentId: `pane-${index}`,
				data: `input-${index}`,
			}),
		);
		expect(result.acks).toContainEqual({
			attachmentId: `pane-${index}`,
			sequence: 42,
		});
	}
});
