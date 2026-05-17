import { describe, expect, it, vi } from "vitest";
import { CodexBridgeRuntime } from "./codex-sdk-bridge-runtime.mjs";

class FakeCodex {
	constructor(eventsByThread = []) {
		this.eventsByThread = eventsByThread;
		this.started = [];
		this.resumed = [];
	}

	startThread(options) {
		const thread = new FakeThread(this.eventsByThread.shift() ?? []);
		this.started.push({ options, thread });
		return thread;
	}

	resumeThread(id, options) {
		const thread = new FakeThread(this.eventsByThread.shift() ?? []);
		this.resumed.push({ id, options, thread });
		return thread;
	}
}

class FakeThread {
	constructor(events) {
		this.events = events;
		this.runCalls = [];
	}

	async runStreamed(input, options) {
		this.runCalls.push({ input, options });
		return {
			events: makeEventStream(this.events, options.signal),
		};
	}
}

async function* makeEventStream(events, signal) {
	for (const event of events) {
		if (event === "throw-error") {
			throw new Error("bad auth");
		}
		if (event instanceof Error) {
			throw event;
		}
		if (event === "wait-for-abort") {
			await new Promise((_, reject) => {
				signal.addEventListener("abort", () => reject(new Error("aborted")), {
					once: true,
				});
			});
		}
		yield event;
	}
}

function makeRuntime(codex) {
	const emitted = [];
	const exits = [];
	const runtime = new CodexBridgeRuntime({
		codex,
		emit: (obj) => emitted.push(obj),
		writeError: vi.fn(),
		exit: (code) => exits.push(code),
		cwd: "/repo",
	});
	return { runtime, emitted, exits };
}

function makeFsApi() {
	const calls = {
		mkdtemp: [],
		chmod: [],
		writeFile: [],
		rm: [],
	};
	return {
		calls,
		api: {
			mkdtemp: vi.fn(async (prefix) => {
				calls.mkdtemp.push(prefix);
				return `${prefix}secure`;
			}),
			chmod: vi.fn(async (target, mode) => {
				calls.chmod.push({ target, mode });
			}),
			writeFile: vi.fn(async (target, data, options) => {
				calls.writeFile.push({ target, data, options });
			}),
			rm: vi.fn(async (target, options) => {
				calls.rm.push({ target, options });
			}),
		},
	};
}

async function waitFor(predicate) {
	const startedAt = Date.now();
	while (!predicate()) {
		if (Date.now() - startedAt > 500) {
			throw new Error("condition was not met");
		}
		await new Promise((resolve) => setTimeout(resolve, 5));
	}
}

describe("CodexBridgeRuntime", () => {
	it("initializes a new thread and does not emit a hardcoded supported_models list", () => {
		const codex = new FakeCodex();
		const { runtime, emitted } = makeRuntime(codex);

		runtime.handleCommand({
			type: "init",
			cwd: "/repo",
			permissionMode: "acceptEdits",
			model: "gpt-5.4",
		});
		runtime.close();

		expect(codex.started[0].options).toMatchObject({
			workingDirectory: "/repo",
			model: "gpt-5.4",
			approvalPolicy: "never",
			sandboxMode: "workspace-write",
		});
		// Codex のモデル一覧は起動時 CLI 同期で config.toml に反映するため、
		// bridge からは supported_models を emit しない。
		expect(
			emitted.some((m) => m.type === "supported_models"),
		).toBe(false);
		expect(emitted[0]).toMatchObject({
			type: "session_ready",
			session_id: null,
			initialized: true,
		});
	});

	it("passes the Rust-provided Codex CLI path to the lazy SDK factory", () => {
		const calls = [];
		const codex = new FakeCodex();
		const emitted = [];
		const runtime = new CodexBridgeRuntime({
			codexFactory: ({ cliPath }) => {
				calls.push(cliPath);
				return codex;
			},
			emit: (obj) => emitted.push(obj),
			writeError: vi.fn(),
			exit: vi.fn(),
			cwd: "/repo",
		});

		runtime.handleCommand({
			type: "init",
			cwd: "/repo",
			codexCliPath: "/usr/local/bin/codex",
		});
		runtime.close();

		expect(calls).toEqual(["/usr/local/bin/codex"]);
		expect(codex.started).toHaveLength(1);
		expect(
			emitted.some((m) => m.type === "supported_models"),
		).toBe(false);
		expect(emitted[0]).toMatchObject({
			type: "session_ready",
			session_id: null,
			initialized: true,
		});
	});

	it("emits session_ready during init when resuming an existing thread", () => {
		const codex = new FakeCodex();
		const { runtime, emitted } = makeRuntime(codex);

		runtime.handleCommand({
			type: "init",
			cwd: "/repo",
			sessionId: "old-thread",
		});
		runtime.close();

		expect(codex.resumed[0]).toMatchObject({ id: "old-thread" });
		expect(emitted).toContainEqual({
			type: "session_ready",
			session_id: "old-thread",
			initialized: true,
		});
	});

	it("runs a message and emits Codex events as bridge JSON", async () => {
		const codex = new FakeCodex([
			[
				{ type: "thread.started", thread_id: "thread-1" },
				{
					type: "item.completed",
					item: { id: "item-1", type: "agent_message", text: "hello" },
				},
				{ type: "turn.completed", usage: { input_tokens: 1, output_tokens: 2 } },
			],
		]);
		const { runtime, emitted, exits } = makeRuntime(codex);

		runtime.handleCommand({ type: "init", cwd: "/repo" });
		runtime.handleCommand({ type: "message", prompt: "hi" });
		await waitFor(() => emitted.some((e) => e.type === "turn_complete"));
		runtime.handleCommand({ type: "close" });
		await runtime.completion;

		expect(codex.started[0].thread.runCalls[0].input).toBe("hi");
		expect(emitted).toContainEqual({
			type: "session_ready",
			session_id: "thread-1",
		});
		expect(emitted).toContainEqual({
			type: "assistant",
			message: {
				role: "assistant",
				content: [{ type: "text", text: "hello" }],
			},
		});
		expect(emitted).toContainEqual({
			type: "turn_complete",
			session_id: "thread-1",
			exit_code: 0,
		});
		expect(exits).toContain(0);
	});

	it("applies setModel to the next thread options", () => {
		const codex = new FakeCodex();
		const { runtime } = makeRuntime(codex);

		runtime.handleCommand({ type: "init", cwd: "/repo" });
		runtime.handleCommand({ type: "setModel", modelId: "gpt-5.3-codex" });
		runtime.close();

		expect(codex.started[1].options).toMatchObject({
			model: "gpt-5.3-codex",
		});
	});

	it("interrupt aborts the active turn and returns to idle completion", async () => {
		const codex = new FakeCodex([["wait-for-abort"]]);
		const { runtime, emitted } = makeRuntime(codex);

		runtime.handleCommand({ type: "init", cwd: "/repo" });
		runtime.handleCommand({ type: "message", prompt: "stop soon" });
		await waitFor(() => codex.started[0].thread.runCalls.length > 0);
		runtime.handleCommand({ type: "interrupt" });
		await waitFor(() => emitted.some((e) => e.type === "turn_complete"));
		runtime.handleCommand({ type: "close" });
		await runtime.completion;

		expect(codex.started[0].thread.runCalls[0].options.signal.aborted).toBe(
			true,
		);
		expect(emitted).toContainEqual({
			type: "turn_complete",
			session_id: null,
			exit_code: 0,
		});
	});

	it("logs permission responses because Codex SDK does not expose permission request events", () => {
		const codex = new FakeCodex();
		const { runtime } = makeRuntime(codex);

		runtime.handleCommand({
			type: "permission_response",
			request_id: "perm-1",
			result: { behavior: "allow" },
		});

		expect(runtime.writeError).toHaveBeenCalledWith(
			expect.stringContaining("permission responses are not supported"),
		);
	});

	it("stores image attachments in private temp dirs and removes the dir after the turn", async () => {
		const fs = makeFsApi();
		const codex = new FakeCodex([
			[
				{ type: "thread.started", thread_id: "thread-1" },
				{ type: "turn.completed", usage: { input_tokens: 1, output_tokens: 1 } },
			],
		]);
		const emitted = [];
		const exits = [];
		const runtime = new CodexBridgeRuntime({
			codex,
			emit: (obj) => emitted.push(obj),
			writeError: vi.fn(),
			exit: (code) => exits.push(code),
			cwd: "/repo",
			tmpDir: "/tmp",
			fsApi: fs.api,
			now: () => 100,
			random: () => 0.5,
		});

		runtime.handleCommand({ type: "init", cwd: "/repo" });
		runtime.handleCommand({
			type: "message",
			prompt: "inspect",
			images: [{ data: Buffer.from("x").toString("base64"), mediaType: "image/png" }],
		});
		await waitFor(() => emitted.some((e) => e.type === "turn_complete"));
		runtime.handleCommand({ type: "close" });
		await runtime.completion;

		expect(fs.calls.mkdtemp[0]).toBe("/tmp/releash-codex-");
		expect(fs.calls.chmod[0]).toEqual({
			target: "/tmp/releash-codex-secure",
			mode: 0o700,
		});
		expect(fs.calls.writeFile[0]).toMatchObject({
			target: "/tmp/releash-codex-secure/100-i.png",
			options: { mode: 0o600 },
		});
		expect(codex.started[0].thread.runCalls[0].input).toEqual([
			{ type: "text", text: "inspect" },
			{ type: "local_image", path: "/tmp/releash-codex-secure/100-i.png" },
		]);
		expect(fs.calls.rm[0]).toEqual({
			target: "/tmp/releash-codex-secure",
			options: { recursive: true, force: true },
		});
	});

	it("exits non-zero after SDK errors so Rust can respawn the bridge", async () => {
		const codex = new FakeCodex([["throw-error"]]);
		const { runtime, emitted, exits } = makeRuntime(codex);

		runtime.handleCommand({ type: "init", cwd: "/repo", sessionId: "old-thread" });
		runtime.handleCommand({ type: "message", prompt: "hi" });
		await runtime.completion;

		expect(emitted).toContainEqual({
			type: "error",
			message: "bad auth",
			clear_session_id: true,
		});
		expect(emitted).toContainEqual({
			type: "turn_complete",
			session_id: "old-thread",
			exit_code: 1,
		});
		expect(exits).toEqual([1]);
	});

	it("starts a new thread and continues the same message when resume fails", async () => {
		const codex = new FakeCodex([
			[new Error("thread not found: old-thread")],
			[
				{ type: "thread.started", thread_id: "new-thread" },
				{
					type: "item.completed",
					item: { id: "item-1", type: "agent_message", text: "recovered" },
				},
				{ type: "turn.completed", usage: { input_tokens: 1, output_tokens: 1 } },
			],
		]);
		const { runtime, emitted, exits } = makeRuntime(codex);

		runtime.handleCommand({ type: "init", cwd: "/repo", sessionId: "old-thread" });
		runtime.handleCommand({ type: "message", prompt: "please continue" });
		await waitFor(() => emitted.some((e) => e.type === "turn_complete"));
		runtime.handleCommand({ type: "close" });
		await runtime.completion;

		expect(codex.resumed[0]).toMatchObject({ id: "old-thread" });
		expect(codex.started).toHaveLength(1);
		expect(codex.started[0].thread.runCalls[0].input).toBe("please continue");
		expect(emitted).toContainEqual({
			type: "session_cleared",
			session_id: "old-thread",
			reason: "resume_failed",
		});
		expect(emitted).toContainEqual({
			type: "session_ready",
			session_id: "new-thread",
		});
		expect(emitted).toContainEqual({
			type: "turn_complete",
			session_id: "new-thread",
			exit_code: 0,
		});
		expect(exits).toContain(0);
	});

	it("exits non-zero after turn.failed events so Rust can respawn the bridge", async () => {
		const codex = new FakeCodex([
			[{ type: "turn.failed", error: { message: "bad auth" } }],
		]);
		const { runtime, emitted, exits } = makeRuntime(codex);

		runtime.handleCommand({ type: "init", cwd: "/repo", sessionId: "old-thread" });
		runtime.handleCommand({ type: "message", prompt: "hi" });
		await runtime.completion;

		expect(emitted).toContainEqual({
			type: "error",
			message: "bad auth",
			clear_session_id: true,
		});
		expect(emitted).toContainEqual({
			type: "turn_complete",
			session_id: "old-thread",
			exit_code: 1,
		});
		expect(exits).toEqual([1]);
	});

	it("close exits the message loop", async () => {
		const codex = new FakeCodex();
		const { runtime, exits } = makeRuntime(codex);

		runtime.handleCommand({ type: "init", cwd: "/repo" });
		runtime.handleCommand({ type: "close" });
		await runtime.completion;

		expect(exits).toEqual([0]);
	});
});
