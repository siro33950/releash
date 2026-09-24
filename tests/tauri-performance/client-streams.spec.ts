import { mkdir, readFile, writeFile } from "node:fs/promises";
import { existsSync } from "node:fs";
import { join } from "node:path";
import { connectPerformanceClient } from "../helpers/performance-daemon.mjs";

declare global {
	interface Window {
		__CLIENT_STREAM_REQUESTS__: Array<{
			method: string;
			url: string;
			elapsedMs: number;
			status: number;
		}>;
	}
}

const directory = process.env.RELEASH_CLIENT_STREAMS_DIRECTORY!;
const artifactDirectory = "performance-results/client-streams-macos";
const paths = Array.from({ length: 6 }, (_, index) =>
	join(directory, `pane-${index}`),
);

async function selectPane(index: number) {
	const node = await $(`button[aria-label^="streams-pane-${index},"]`);
	await node.waitForDisplayed();
	await node.click();
	const pane = await $(`[data-testid="worktree-pane-${paths[index]}"]`);
	await pane.waitForDisplayed();
	return pane.$('[data-testid="right-bottom-content"] .xterm-helper-textarea');
}

async function buffers() {
	return browser.execute(() =>
		Object.values(window.__RELEASH_TERMINAL_BUFFER_READERS__ ?? {}).map(
			(read) => read().text,
		),
	);
}

describe("macOS WKWebView / real Connect daemon", () => {
	afterEach(async () => {
		await mkdir(artifactDirectory, { recursive: true });
		await browser.saveScreenshot(
			join(artifactDirectory, "client-streams-macos.png"),
		);
	});
	it("最大5paneとpushを保持したまま入力・継続出力・状態再取得が進む", async () => {
		await browser.tauri.switchWindow("main");
		await browser.setTimeout({ script: 5_000 });
		await browser.waitUntil(
			() => browser.execute(() => Boolean(window.__RELEASH_INVOKE_CLIENT__)),
			{ timeout: 30_000 },
		);
		const environment = await browser.execute(() => {
			const nativeFetch = window.fetch;
			window.__CLIENT_STREAM_REQUESTS__ = [];
			window.fetch = async (input, init) => {
				const url = input instanceof Request ? input.url : String(input);
				const started = performance.now();
				const response = await nativeFetch(input, init);
				if (url.includes("/releash.client.v1.ClientService/"))
					window.__CLIENT_STREAM_REQUESTS__.push({
						method: url.split("/").pop()!,
						url,
						elapsedMs: performance.now() - started,
						status: response.status,
					});
				return response;
			};
			return { origin: location.origin, userAgent: navigator.userAgent };
		});
		expect(environment.origin).toBe("tauri://localhost");
		expect(environment.userAgent).toContain("AppleWebKit");
		expect(environment.userAgent).not.toContain("Chrome");
		await browser.waitUntil(
			() => existsSync(join(directory, "data/client-api.json")),
			{
				timeout: 30_000,
				timeoutMsg: "isolated daemon discovery was not created",
			},
		);
		const discovery = JSON.parse(
			await readFile(join(directory, "data/client-api.json"), "utf8"),
		);
		const client = await connectPerformanceClient(discovery);
		for (let index = 0; index < paths.length; index++) {
			await client.addRepoPath({ path: paths[index] });
			await browser.execute(
				async ({ worktreePath, name }) => {
					const invoke = window.__RELEASH_INVOKE_CLIENT__!;
					const sessionId = await invoke("create_agent_session", {
						workspaceIdentity: worktreePath,
						worktreePath,
						provider: "codex",
						rows: 24,
						cols: 80,
						callerRequestId: crypto.randomUUID(),
					});
					const nodeId = await invoke("get_workspace_session_node_id", {
						worktreePath,
						sessionId,
					});
					if (!nodeId) throw new Error("Session node was not created");
					await invoke("rename_workspace_session_node", {
						worktreePath,
						nodeId,
						name,
					});
				},
				{ worktreePath: paths[index], name: `streams-pane-${index}` },
			);
			await browser.waitUntil(async () => {
				const snapshot = await client.listBranchesWithStatusSnapshot({
					repoPath: paths[index],
				});
				return (
					!snapshot.loading &&
					snapshot.worktreeDisplayGroups?.workingAreas?.items.some(
						(branch) => branch.worktreePath === paths[index],
					)
				);
			});
			await $(`button[aria-label="Refresh pane-${index}"]`).click();
			await $(`[data-testid="worktree-item-pane-${index}"]`).waitForDisplayed();
			await (await selectPane(index)).waitForExist();
		}
		expect(await $$('[data-testid^="worktree-pane-"]')).toHaveLength(5);
		expect(
			await $(`[data-testid="worktree-pane-${paths[0]}"]`).isExisting(),
		).toBe(false);
		for (let index = 1; index <= 5; index++) {
			const script = [
				"import select, sys, termios",
				"settings = termios.tcgetattr(0)",
				"settings[3] &= ~termios.ECHO",
				"termios.tcsetattr(0, termios.TCSANOW, settings)",
				"count = 0",
				"while True:",
				"    if select.select([sys.stdin], [], [], 0.1)[0]:",
				`        print('received:${index}:' + sys.stdin.readline().strip(), flush=True)`,
				`    print('tick:${index}:' + str(count), flush=True)`,
				"    count += 1",
			].join("\n");
			const encoded = Buffer.from(script).toString("base64");
			const input = await selectPane(index);
			await input.addValue(
				`python3 -u -c "import base64;exec(base64.b64decode('${encoded}'))"\r`,
			);
			await browser.waitUntil(async () =>
				(await buffers()).some((text) => text.includes(`tick:${index}:`)),
			);
		}
		const ticks = async () => {
			const text = (await buffers()).join("\n");
			return Array.from({ length: 5 }, (_, index) =>
				Math.max(
					-1,
					...Array.from(
						text.matchAll(new RegExp(`tick:${index + 1}:(\\d+)`, "g")),
						(match) => Number(match[1]),
					),
				),
			);
		};
		const before = await ticks();
		const unaryMs: number[] = [];
		for (let round = 0; round < 3; round++) {
			for (let index = 1; index <= 5; index++) {
				await (await selectPane(index)).addValue(`round-${round}\r`);
				await browser.waitUntil(
					async () =>
						(await buffers()).some((text) =>
							text.includes(`received:${index}:round-${round}`),
						),
					{ timeout: 5_000 },
				);
			}
			const started = Date.now();
			const state = await browser.execute(() =>
				window.__RELEASH_INVOKE_CLIENT__!("get_workspaces").then((snapshot) => snapshot.repositories.map((repo) => repo.path)),
			);
			unaryMs.push(Date.now() - started);
			for (const path of paths) expect(state).toContain(path);
			const surfaces = await browser.execute(
				(workspaces) =>
					Promise.all(
						workspaces.map((workspacePath) =>
							window.__RELEASH_INVOKE_CLIENT__!("get_terminal_surface", {
								owner: { kind: "workspace", workspacePath },
							}),
						),
					),
				paths.slice(1),
			);
			expect(surfaces).toHaveLength(5);
			for (const surface of surfaces) expect(surface.is_exited).toBe(false);
		}
		await client.removeRepoPath({ path: paths[0] });
		await $(`[data-testid="worktree-item-pane-0"]`).waitForExist({
			reverse: true,
			timeout: 5_000,
		});
		const current = await browser.execute(() =>
			window.__RELEASH_INVOKE_CLIENT__!("get_workspaces").then((snapshot) => snapshot.repositories.map((repo) => repo.path)),
		);
		expect(current).not.toContain(paths[0]);
		const after = await ticks();
		for (let index = 0; index < 5; index++)
			expect(after[index]).toBeGreaterThan(before[index]);
		expect(await $$('[data-testid^="worktree-pane-"]')).toHaveLength(5);
		const requests = await browser.execute(() =>
			window.__CLIENT_STREAM_REQUESTS__.filter((request) =>
				[
					"WriteTerminalSurface",
					"AckTerminalSurfaceOutput",
					"GetWorkspaces",
					"GetTerminalSurface",
				].includes(request.method),
			),
		);
		for (const request of requests) {
			expect(request.url).toMatch(
				new RegExp(`^http://127\\.0\\.0\\.1:${discovery.port}/`),
			);
			expect(request.status).toBe(200);
			expect(request.elapsedMs).toBeLessThan(5_000);
		}
		expect(
			requests.filter((request) => request.method === "WriteTerminalSurface")
				.length,
		).toBeGreaterThanOrEqual(20);
		expect(
			requests.some((request) => request.method === "AckTerminalSurfaceOutput"),
		).toBe(true);
		await mkdir(artifactDirectory, { recursive: true });
		await writeFile(
			join(artifactDirectory, "client-streams-macos.json"),
			JSON.stringify(
				{
					environment,
					panes: 5,
					inputRounds: 3,
					ticksBefore: before,
					ticksAfter: after,
					unaryMs,
					requests,
				},
				null,
				2,
			),
		);
	});
});
