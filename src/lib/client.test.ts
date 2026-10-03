import type { MessageInitShape } from "@bufbuild/protobuf";
import { Code, type HandlerContext } from "@connectrpc/connect";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
	CommandErrorSchema,
	type StartStateSubscriptionRequest,
	type StateSubscriptionEventSchema,
} from "@/generated/client_pb";

vi.unmock("@/lib/client");
let invoke: typeof import("@tauri-apps/api/core").invoke;
let ConnectError: typeof import("@connectrpc/connect").ConnectError;
let connectFixture: typeof import("@/test/connect").connectFixture;
let getErrorMessage: typeof import("./errorMessage").getErrorMessage;
let firstState: typeof import("./client").firstState;
let invokeClient: typeof import("./client").invokeClient;
let subscribeState: typeof import("./client").subscribeState;
let checkTransitions: () => void;
const requestUrl = (input: RequestInfo | URL) =>
	input instanceof Request ? input.url : input.toString();
beforeEach(async () => {
	vi.resetModules();
	({ invoke } = await import("@tauri-apps/api/core"));
	({ ConnectError } = await import("@connectrpc/connect"));
	({ connectFixture } = await import("@/test/connect"));
	({ getErrorMessage } = await import("./errorMessage"));
	({ firstState, invokeClient, subscribeState } = await import("./client"));
	const { getConnectionState, onConnectionStateChange } = await import(
		"./client"
	);
	const phases = [getConnectionState()];
	const release = onConnectionStateChange(() =>
		phases.push(getConnectionState()),
	);
	checkTransitions = () => {
		release();
		const allowed: Record<string, string[]> = {
			IDLE: ["CONNECTING", "SHUTDOWN"],
			CONNECTING: ["CONNECTING", "READY", "TRANSIENT_FAILURE", "SHUTDOWN"],
			READY: ["READY", "TRANSIENT_FAILURE", "SHUTDOWN"],
			TRANSIENT_FAILURE: ["CONNECTING", "SHUTDOWN"],
			SHUTDOWN: [],
		};
		for (let index = 1; index < phases.length; index++)
			expect(allowed[phases[index - 1]]).toContain(phases[index]);
	};
});
afterEach(async () => {
	window.dispatchEvent(new Event("pagehide"));
	await new Promise((resolve) => setTimeout(resolve, 0));
	checkTransitions();
	vi.unstubAllGlobals();
});

it("生成RPCで引数と結果を変換する", async () => {
	const read = vi.fn(() => ({ value: true }));
	const fixture = connectFixture({ addRepoPath: read });
	await expect(invokeClient("add_repo_path", { path: "/repo" })).resolves.toBe(
		true,
	);
	expect(read).toHaveBeenCalledOnce();
	expect(
		fixture.requests.map((request) => new URL(request.url).pathname),
	).toEqual([
		"/releash.client.v1.ClientService/GetServerInfo",
		"/releash.client.v1.ClientService/AddRepoPath",
	]);
});

it("応答未到達の変更要求では再接続せず同じ変更を再送しない", async () => {
	const { getClient } = await import("./client");
	const mutate = vi.fn(() => {
		throw new ConnectError("response lost", Code.Unavailable);
	});
	const fixture = connectFixture({ updateCrashReporting: mutate });
	const client = await getClient();
	await expect(
		invokeClient("update_crash_reporting", { enabled: true }),
	).rejects.toBeInstanceOf(ConnectError);
	expect(await getClient()).toBe(client);
	expect(mutate).toHaveBeenCalledOnce();
	expect(
		fixture.requests.every(
			(request) => !/Operation|Query|AckRequest/.test(request.url),
		),
	).toBe(true);
});

it("送信失敗のPromiseを終了し元要求を保持して再送しない", async () => {
	const fixture = connectFixture();
	const original = fixture.fetch.getMockImplementation();
	if (!original) throw new Error("Missing fetch fixture");
	let writes = 0;
	fixture.fetch.mockImplementation(async (input, init) => {
		if (requestUrl(input).endsWith("/UpdateExternalEditor")) {
			writes++;
			throw new TypeError("network failure");
		}
		return original(input, init);
	});
	await expect(
		invokeClient("update_external_editor", { editor: "vim" }),
	).rejects.toBeDefined();
	expect(writes).toBe(1);
});

it("CONNECTING中の単発呼び出しはREADYまで待ってから送る", async () => {
	const { getConnectionState } = await import("./client");
	const read = vi.fn(() => ({ value: true }));
	connectFixture({ addRepoPath: read });
	const original = vi.mocked(invoke).getMockImplementation();
	if (!original) throw new Error("Missing endpoint fixture");
	let release!: (endpoint: unknown) => void;
	vi.mocked(invoke).mockImplementation((command, args) =>
		command === "get_client_endpoint"
			? new Promise((resolve) => {
					release = resolve;
				})
			: original(command, args),
	);
	const first = invokeClient("add_repo_path", { path: "/one" });
	const second = invokeClient("add_repo_path", { path: "/two" });
	expect(getConnectionState()).toBe("CONNECTING");
	expect(read).not.toHaveBeenCalled();
	release({
		url: "http://127.0.0.1:9829",
		token: "client-token",
		launchId: "launch",
	});
	await expect(Promise.all([first, second])).resolves.toEqual([true, true]);
	expect(getConnectionState()).toBe("READY");
	expect(read).toHaveBeenCalledTimes(2);
});

it("接続先の取得が20秒を超えると失敗し次の単発呼び出しも即座に失敗する", async () => {
	const { getConnectionState } = await import("./client");
	vi.useFakeTimers();
	try {
		vi.mocked(invoke).mockImplementation(() => new Promise(() => {}));
		vi.mocked(invoke).mockClear();
		const pending = invokeClient("add_repo_path", { path: "/repo" }).catch(
			(error) => error,
		);
		expect(getConnectionState()).toBe("CONNECTING");
		await vi.advanceTimersByTimeAsync(20_000);
		expect(await pending).toMatchObject({
			message: "Daemon connection timed out",
		});
		expect(getConnectionState()).toBe("TRANSIENT_FAILURE");
		await expect(
			invokeClient("add_repo_path", { path: "/repo" }),
		).rejects.toThrow("Daemon connection is TRANSIENT_FAILURE");
		expect(invoke).toHaveBeenCalledTimes(1);
	} finally {
		vi.useRealTimers();
	}
});

it.each(["GetServerInfo", "validate_daemon_connection"])(
	"%sの応答が20秒ない場合は接続の確立を失敗させる",
	async (step) => {
		const { getConnectionState } = await import("./client");
		vi.useFakeTimers();
		try {
			const fixture = connectFixture();
			if (step === "GetServerInfo") {
				const original = fixture.fetch.getMockImplementation();
				if (!original) throw new Error("Missing fetch fixture");
				fixture.fetch.mockImplementation((input, init) =>
					requestUrl(input).endsWith("/GetServerInfo")
						? new Promise(() => {})
						: original(input, init),
				);
			} else {
				const original = vi.mocked(invoke).getMockImplementation();
				if (!original) throw new Error("Missing IPC fixture");
				vi.mocked(invoke).mockImplementation((command, args) =>
					command === "validate_daemon_connection"
						? new Promise(() => {})
						: original(command, args),
				);
			}
			const pending = invokeClient("add_repo_path", { path: "/repo" }).catch(
				(error) => error,
			);
			expect(getConnectionState()).toBe("CONNECTING");
			await vi.advanceTimersByTimeAsync(20_000);
			expect(await pending).toMatchObject({
				message: "Daemon connection timed out",
			});
			expect(getConnectionState()).toBe("TRANSIENT_FAILURE");
		} finally {
			vi.useRealTimers();
		}
	},
);

it("失効した接続先取得が後から完了しても現在の接続を変えない", async () => {
	vi.useFakeTimers();
	try {
		connectFixture({ addRepoPath: () => ({ value: true }) });
		const original = vi.mocked(invoke).getMockImplementation();
		if (!original) throw new Error("Missing IPC fixture");
		let releaseFirst!: (value: unknown) => void;
		let attempts = 0;
		vi.mocked(invoke).mockImplementation((command, args) => {
			if (command === "get_client_endpoint" && attempts++ === 0)
				return new Promise((resolve) => {
					releaseFirst = resolve;
				});
			return original(command, args);
		});
		const first = invokeClient("add_repo_path", { path: "/first" }).catch(
			(error) => error,
		);
		await vi.advanceTimersByTimeAsync(20_000);
		expect(await first).toMatchObject({
			message: "Daemon connection timed out",
		});
		await vi.advanceTimersByTimeAsync(1_000);
		const { getConnectionState } = await import("./client");
		expect(getConnectionState()).toBe("READY");
		releaseFirst({
			url: "http://127.0.0.1:9829",
			token: "old-token",
			launchId: "old",
		});
		await vi.advanceTimersByTimeAsync(0);
		expect(getConnectionState()).toBe("READY");
		await expect(
			invokeClient("add_repo_path", { path: "/current" }),
		).resolves.toBe(true);
		expect(attempts).toBe(2);
	} finally {
		vi.useRealTimers();
	}
});

it("SHUTDOWN中の単発呼び出しは接続を始めず失敗する", async () => {
	const { getConnectionState } = await import("./client");
	window.dispatchEvent(new Event("pagehide"));
	vi.mocked(invoke).mockClear();
	expect(getConnectionState()).toBe("SHUTDOWN");
	await expect(
		invokeClient("add_repo_path", { path: "/repo" }),
	).rejects.toThrow("Daemon connection is SHUTDOWN");
	expect(invoke).not.toHaveBeenCalled();
	window.dispatchEvent(
		new PageTransitionEvent("pageshow", { persisted: true }),
	);
	expect(getConnectionState()).toBe("SHUTDOWN");
	await expect(
		invokeClient("add_repo_path", { path: "/repo" }),
	).rejects.toThrow("Daemon connection is SHUTDOWN");
	expect(invoke).not.toHaveBeenCalled();
});

it("復帰可能なページはSHUTDOWNを経ずに再接続する", async () => {
	const { getConnectionState, onConnectionStateChange } = await import(
		"./client"
	);
	const fixture = connectFixture({ addRepoPath: () => ({ value: true }) });
	await invokeClient("add_repo_path", { path: "/first" });
	const phases: string[] = [];
	const release = onConnectionStateChange(() =>
		phases.push(getConnectionState()),
	);
	window.dispatchEvent(
		new PageTransitionEvent("pagehide", { persisted: true }),
	);
	expect(getConnectionState()).toBe("TRANSIENT_FAILURE");
	window.dispatchEvent(
		new PageTransitionEvent("pageshow", { persisted: true }),
	);
	await expect(
		invokeClient("add_repo_path", { path: "/second" }),
	).resolves.toBe(true);
	expect(phases).toEqual(["TRANSIENT_FAILURE", "CONNECTING", "READY"]);
	expect(
		fixture.requests.filter((request) =>
			request.url.endsWith("/GetServerInfo"),
		),
	).toHaveLength(2);
	release();
});

it("設定保存が期限超過してもPromiseを終了し同じ変更を再送しない", async () => {
	vi.useFakeTimers();
	try {
		const save = vi.fn(async (_: unknown, context: HandlerContext) => {
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
			return {};
		});
		connectFixture({ updateWorkflowConfig: save });
		const result = invokeClient("update_workflow_config", {
			workflow: { approval_auto_approve: true },
		}).then(
			() => null,
			(error) => error,
		);
		await vi.waitFor(() => expect(save).toHaveBeenCalledOnce());
		await vi.advanceTimersByTimeAsync(120_001);
		expect(await result).toMatchObject({ code: Code.DeadlineExceeded });
		expect(save).toHaveBeenCalledOnce();
	} finally {
		window.dispatchEvent(new Event("pagehide"));
		vi.useRealTimers();
	}
});

it("遅延した単発要求の失敗が接続と進行中の要求を破棄しない", async () => {
	const { getClient } = await import("./client");
	let release!: () => void;
	const read = vi.fn(async () => {
		await new Promise<void>((resolve) => {
			release = resolve;
		});
		return { value: true };
	});
	const fixture = connectFixture({ addRepoPath: read });
	const original = fixture.fetch.getMockImplementation();
	if (!original) throw new Error("Missing fixture implementation");
	let fail!: () => void;
	fixture.fetch.mockImplementation(async (input, init) => {
		if (requestUrl(input).endsWith("/UpdateExternalEditor"))
			return new Promise<Response>((_, reject) => {
				fail = () => reject(new TypeError("late failure"));
			});
		return original(input, init);
	});
	const old = invokeClient("update_external_editor", { editor: "vim" }).catch(
		(error) => error,
	);
	await vi.waitFor(() => expect(fail).toBeDefined());
	const current = await getClient();
	const pending = invokeClient("add_repo_path", { path: "/repo" });
	await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
	fail();
	expect(await old).toBeInstanceOf(ConnectError);
	expect(await getClient()).toBe(current);
	await release();
	await expect(pending).resolves.toBe(true);
});

function commandError(
	code: string,
	message: string,
	status = Code.FailedPrecondition,
) {
	return new ConnectError("Command failed", status, undefined, [
		{
			desc: CommandErrorSchema,
			value: { variant: { case: "coded", value: { code, message } } },
		},
	]);
}

it("実adapterがCommandError detailのcodeとmessageを保持する", async () => {
	const { getClient } = await import("./client");
	connectFixture({
		addRepoPath: () => {
			throw commandError("REPOSITORY_UNAVAILABLE", "Repository is unavailable");
		},
	});
	const client = await getClient();
	const error = await invokeClient("add_repo_path", { path: "/repo" }).catch(
		(error) => error,
	);
	expect(error).toStrictEqual({
		code: "REPOSITORY_UNAVAILABLE",
		message: "Repository is unavailable",
	});
	expect(error).not.toBeInstanceOf(Error);
	expect(await getClient()).toBe(client);
});

it("失効したterminal attachmentのcodeを入力側へ届ける", async () => {
	connectFixture({
		writeTerminalSurface: () => {
			throw commandError(
				"STALE_TERMINAL_ATTACHMENT",
				"Terminal input could not be sent. Try again.",
			);
		},
	});
	await expect(
		invokeClient("write_terminal_surface", {
			owner: { kind: "workspace", workspacePath: "/repo" },
			attachmentId: "old-attachment",
			sequence: 0,
			data: "x",
		}),
	).rejects.toEqual({
		code: "STALE_TERMINAL_ATTACHMENT",
		message: "Terminal input could not be sent. Try again.",
	});
});

it("進行中要求の上限超過は理由を表示し変更要求を再送しない", async () => {
	const { getClient } = await import("./client");
	const update = vi.fn(() => {
		throw commandError(
			"CLIENT_REQUEST_LIMIT",
			"Too many pending client commands",
			Code.ResourceExhausted,
		);
	});
	connectFixture({ updateExternalEditor: update });
	const client = await getClient();
	const error = await invokeClient("update_external_editor", {
		editor: "vim",
	}).catch((error) => error);
	expect(error).toStrictEqual({
		code: "CLIENT_REQUEST_LIMIT",
		message: "Too many pending client commands",
	});
	expect(getErrorMessage(error)).toBe("Too many pending client commands");
	expect(await getClient()).toBe(client);
	expect(update).toHaveBeenCalledOnce();
});

it.each([
	Code.Unavailable,
	Code.ResourceExhausted,
	Code.InvalidArgument,
	Code.NotFound,
	Code.PermissionDenied,
	Code.DeadlineExceeded,
	Code.Canceled,
	Code.Unknown,
	Code.Internal,
	Code.Unauthenticated,
	Code.AlreadyExists,
	Code.FailedPrecondition,
	Code.Aborted,
	Code.OutOfRange,
	Code.Unimplemented,
	Code.DataLoss,
])("RPC失敗code %sは共有接続と進行中のRPCを中断しない", async (code) => {
	const { getClient } = await import("./client");
	let release!: () => void;
	const read = vi.fn(async () => {
		await new Promise<void>((resolve) => {
			release = resolve;
		});
		return { value: true };
	});
	const fixture = connectFixture({
		addRepoPath: read,
		updateExternalEditor: () => {
			throw new ConnectError("request rejected", code);
		},
	});
	const client = await getClient();
	const pending = invokeClient("add_repo_path", { path: "/repo" });
	await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
	const error = await invokeClient("update_external_editor", {
		editor: "vim",
	}).catch((error) => error);
	expect(error).toBeInstanceOf(ConnectError);
	expect(error).toMatchObject({ code, rawMessage: "request rejected" });
	expect(await getClient()).toBe(client);
	expect(
		fixture.requests.find((request) => request.url.endsWith("/AddRepoPath"))
			?.signal.aborted,
	).toBe(false);
	await release();
	await expect(pending).resolves.toBe(true);
});

it("AbortSignal.anyが無くても初回RPCとstreamが開始し個別と接続のabortが効く", async () => {
	vi.stubGlobal(
		"AbortSignal",
		new Proxy(AbortSignal, {
			get: (target, key) =>
				key === "any" ? undefined : Reflect.get(target, key),
		}),
	);
	const { getClient } = await import("./client");
	const started = vi.fn();
	const fixture = connectFixture({
		async *openStateStream(_, context) {
			yield { event: { case: "ready", value: {} } };
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
		},
		startStateSubscription: () => ({}),
		async addRepoPath(_, context) {
			started();
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
			return { value: false };
		},
	});
	expect(AbortSignal.any).toBeUndefined();
	const client = await getClient();
	const release = subscribeState("repository-paths", vi.fn(), vi.fn());
	await vi.waitFor(() =>
		expect(
			fixture.requests.some((request) =>
				request.url.endsWith("/StartStateSubscription"),
			),
		).toBe(true),
	);
	const abort = new AbortController();
	const pending = client
		.addRepoPath({ path: "/repo" }, { signal: abort.signal })
		.catch((error) => error);
	await vi.waitFor(() => expect(started).toHaveBeenCalledOnce());
	abort.abort();
	expect(await pending).toMatchObject({ code: Code.Canceled });
	const streaming = fixture.requests.find((request) =>
		request.url.endsWith("/OpenStateStream"),
	);
	expect(streaming?.signal.aborted).toBe(false);
	window.dispatchEvent(new Event("pagehide"));
	expect(streaming?.signal.aborted).toBe(true);
	release();
});

type StateEvent = MessageInitShape<typeof StateSubscriptionEventSchema>;

function stateFixture(
	start?: (
		request: StartStateSubscriptionRequest,
	) => Promise<Record<string, never>>,
	options: {
		failBeforeReady?: number;
		stop?: (request: { subscriptionId: string }) => Record<string, never>;
		addRepoPath?: () => { value: boolean };
		writeTerminalSurface?: (request: {
			attachmentId?: string;
		}) => Record<string, never>;
	} = {},
) {
	const streams: {
		send: (event: StateEvent) => void;
		fail: (code?: Code) => void;
		signal: AbortSignal;
		openedAt: number;
	}[] = [];
	const starts: StartStateSubscriptionRequest[] = [];
	const reports = vi.fn(
		(_request: { subscriptionId: string; units: number }) => ({}),
	);
	const stops = vi.fn(
		(request: { subscriptionId: string }) => options.stop?.(request) ?? {},
	);
	const fixture = connectFixture({
		...(options.addRepoPath ? { addRepoPath: options.addRepoPath } : {}),
		...(options.writeTerminalSurface
			? { writeTerminalSurface: options.writeTerminalSurface }
			: {}),
		async *openStateStream(_, context) {
			const queue: (StateEvent | Error)[] = [];
			let wake = () => {};
			const push = (item: StateEvent | Error) => {
				queue.push(item);
				wake();
			};
			streams.push({
				send: push,
				fail: (code = Code.Unavailable) =>
					push(new ConnectError("state lost", code)),
				signal: context.signal,
				openedAt: Date.now(),
			});
			if (streams.length <= (options.failBeforeReady ?? 0))
				throw new ConnectError("open failed", Code.Unavailable);
			yield { event: { case: "ready", value: {} } };
			while (!context.signal.aborted) {
				const item = queue.shift();
				if (item instanceof Error) throw item;
				if (item) {
					yield item;
					continue;
				}
				await new Promise<void>((resolve) => {
					wake = resolve;
					context.signal.addEventListener("abort", () => resolve(), {
						once: true,
					});
				});
			}
		},
		startStateSubscription(request) {
			starts.push(request);
			return start?.(request) ?? {};
		},
		stopStateSubscription: stops,
		reportTerminalProcessed: reports,
	});
	const serverInfoRequests = () =>
		fixture.requests.filter((request) => request.url.endsWith("/GetServerInfo"))
			.length;
	return {
		streams,
		starts,
		stops,
		reports,
		serverInfoRequests,
		subscriptionId: (target: string) => {
			const request = [...starts]
				.reverse()
				.find((request) => request.target === target);
			if (!request) throw new Error(`Subscription not started: ${target}`);
			return request.subscriptionId;
		},
	};
}

it("状態のstreamがreadyの直後に切れ続けても待ちが伸びる", async () => {
	vi.useFakeTimers();
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	try {
		const fixture = stateFixture();
		subscribeState("repository-paths", vi.fn(), vi.fn());
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
		const firstFailedAt = Date.now();
		fixture.streams[0].fail();
		await vi.advanceTimersByTimeAsync(1000);
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
		const secondFailedAt = Date.now();
		fixture.streams[1].fail();
		await vi.advanceTimersByTimeAsync(1600);
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(3));
		expect([
			fixture.streams[1].openedAt - firstFailedAt,
			fixture.streams[2].openedAt - secondFailedAt,
		]).toEqual([1000, 1600]);
	} finally {
		window.dispatchEvent(new Event("pagehide"));
		await vi.advanceTimersByTimeAsync(1000);
		vi.useRealTimers();
		vi.restoreAllMocks();
	}
});

it("購読開始が失敗し続けても同じ待ちが伸びる", async () => {
	vi.useFakeTimers();
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	try {
		const fixture = stateFixture(async () => {
			throw new ConnectError("busy", Code.Aborted);
		});
		subscribeState("repository-paths", vi.fn(), vi.fn());
		await vi.advanceTimersByTimeAsync(6000);
		expect(fixture.streams).toHaveLength(4);
		const opened = fixture.streams.map((stream) => stream.openedAt);
		expect([
			opened[1] - opened[0],
			opened[2] - opened[1],
			opened[3] - opened[2],
		]).toEqual([1000, 1600, 2560]);
		expect(fixture.serverInfoRequests()).toBe(1);
	} finally {
		window.dispatchEvent(new Event("pagehide"));
		await vi.advanceTimersByTimeAsync(1000);
		vi.useRealTimers();
		vi.restoreAllMocks();
	}
});

it("ready前の失敗でも待ちが伸びる", async () => {
	vi.useFakeTimers();
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	try {
		const fixture = stateFixture(undefined, { failBeforeReady: 3 });
		subscribeState("repository-paths", vi.fn(), vi.fn());
		await vi.advanceTimersByTimeAsync(6000);
		expect(fixture.streams).toHaveLength(4);
		const opened = fixture.streams.map((stream) => stream.openedAt);
		expect([
			opened[1] - opened[0],
			opened[2] - opened[1],
			opened[3] - opened[2],
		]).toEqual([1000, 1600, 2560]);
	} finally {
		window.dispatchEvent(new Event("pagehide"));
		await vi.advanceTimersByTimeAsync(1000);
		vi.useRealTimers();
		vi.restoreAllMocks();
	}
});

it("つなぎ直してから2分を超えてstreamが続くと次の待ちは1秒に戻る", async () => {
	vi.useFakeTimers();
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	try {
		const fixture = stateFixture();
		subscribeState("repository-paths", vi.fn(), vi.fn());
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
		fixture.streams[0].fail();
		await vi.advanceTimersByTimeAsync(1000);
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
		for (let i = 0; i < 7; i++) {
			await vi.advanceTimersByTimeAsync(19_000);
			fixture.streams[1].send({
				subscriptionId: fixture.subscriptionId("repository-paths"),
				version: { epoch: "boot", sequence: BigInt(i) },
				event: { case: "bookmark", value: {} },
			});
			await vi.advanceTimersByTimeAsync(0);
		}
		await vi.advanceTimersByTimeAsync(1);
		const failedAt = Date.now();
		fixture.streams[1].fail();
		await vi.advanceTimersByTimeAsync(1000);
		await vi.waitFor(() => expect(fixture.streams).toHaveLength(3));
		expect(fixture.streams[2].openedAt - failedAt).toBe(1000);
	} finally {
		window.dispatchEvent(new Event("pagehide"));
		await vi.advanceTimersByTimeAsync(1000);
		vi.useRealTimers();
		vi.restoreAllMocks();
	}
});

it.each([Code.Unavailable, Code.Aborted, Code.ResourceExhausted])(
	"購読の開始がcode %sで失敗したら接続を保ってstreamを開き直す",
	async (code) => {
		const start = vi
			.fn<() => Promise<Record<string, never>>>()
			.mockRejectedValueOnce(new ConnectError("busy", code))
			.mockResolvedValue({});
		const fixture = stateFixture(start);
		const error = vi.fn();
		subscribeState("repository-paths", vi.fn(), error);
		await vi.waitFor(() => expect(fixture.streams).toHaveLength(2), {
			timeout: 3000,
		});
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
		expect(fixture.streams[0].signal.aborted).toBe(true);
		expect(fixture.starts.map((request) => request.target)).toEqual([
			"repository-paths",
			"repository-paths",
		]);
		expect(error).not.toHaveBeenCalled();
		expect(fixture.serverInfoRequests()).toBe(1);
	},
);

it("購読開始のRESOURCE_EXHAUSTED中もREADYで単発RPCを送る", async () => {
	const { getConnectionState, onConnectionStateChange } = await import(
		"./client"
	);
	const rpc = vi.fn(() => ({ value: true }));
	const start = vi
		.fn<() => Promise<Record<string, never>>>()
		.mockRejectedValueOnce(new ConnectError("busy", Code.ResourceExhausted))
		.mockResolvedValue({});
	const fixture = stateFixture(start, { addRepoPath: rpc });
	const phases: string[] = [];
	const release = onConnectionStateChange(() =>
		phases.push(getConnectionState()),
	);
	try {
		subscribeState("repository-paths", vi.fn(), vi.fn());
		await vi.waitFor(() =>
			expect(fixture.streams[0]?.signal.aborted).toBe(true),
		);
		expect(getConnectionState()).toBe("READY");
		await expect(
			invokeClient("add_repo_path", { path: "/repo" }),
		).resolves.toBe(true);
		expect(rpc).toHaveBeenCalledOnce();
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
			timeout: 3000,
		});
		expect(phases).toEqual(["CONNECTING", "READY"]);
	} finally {
		release();
	}
});

it.each([Code.Unavailable, Code.Aborted, Code.ResourceExhausted])(
	"購読の停止がcode %sで失敗したらstreamを開き直し残った対象だけを開始する",
	async (code) => {
		const fixture = stateFixture(undefined, {
			stop: () => {
				throw new ConnectError("state lost", code);
			},
		});
		const release = subscribeState("repository-paths", vi.fn(), vi.fn());
		subscribeState("providers", vi.fn(), vi.fn());
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
		release();
		await vi.waitFor(() => expect(fixture.streams).toHaveLength(2), {
			timeout: 3000,
		});
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(3));
		expect(fixture.stops).toHaveBeenCalledOnce();
		expect(fixture.starts[2].target).toBe("providers");
		expect(fixture.serverInfoRequests()).toBe(1);
	},
);

const repositoryPaths = (
	subscriptionId: string,
	sequence: number,
	items: string[],
	kind: "snapshot" | "change" = "change",
): StateEvent => ({
	subscriptionId,
	version: { epoch: "boot", sequence: BigInt(sequence) },
	event:
		kind === "snapshot"
			? {
					case: "snapshot",
					value: { value: { case: "repositoryPaths", value: { items } } },
				}
			: {
					case: "change",
					value: {
						payload: { value: { case: "repositoryPaths", value: { items } } },
					},
				},
});

it("状態の購読は同じ対象を一度だけ開始し最初の状態と変更を全ての受け手へ届ける", async () => {
	const fixture = stateFixture();
	const first = vi.fn();
	const second = vi.fn();
	subscribeState("repository-paths", first, vi.fn());
	subscribeState("repository-paths", second, vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	expect(fixture.starts[0].target).toBe("repository-paths");
	expect(fixture.starts[0].version).toBeUndefined();
	fixture.streams[0].send(
		repositoryPaths(
			fixture.subscriptionId("repository-paths"),
			0,
			["/a"],
			"snapshot",
		),
	);
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("repository-paths"),
		version: { epoch: "boot", sequence: 0n },
		event: { case: "bookmark", value: {} },
	});
	fixture.streams[0].send(
		repositoryPaths(fixture.subscriptionId("repository-paths"), 1, [
			"/a",
			"/b",
		]),
	);
	await vi.waitFor(() => {
		expect(first.mock.calls).toEqual([[["/a"]], [["/a", "/b"]]]);
		expect(second.mock.calls).toEqual([[["/a"]], [["/a", "/b"]]]);
	});
	const late = vi.fn();
	subscribeState("repository-paths", late, vi.fn());
	expect(late).toHaveBeenCalledWith(["/a", "/b"]);
	expect(fixture.starts).toHaveLength(1);
});

it("復帰可能なページは状態の購読を再開する", async () => {
	const fixture = stateFixture();
	const receive = vi.fn();
	subscribeState("repository-paths", receive, vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send(
		repositoryPaths(
			fixture.subscriptionId("repository-paths"),
			0,
			["/a"],
			"snapshot",
		),
	);
	await vi.waitFor(() => expect(receive).toHaveBeenLastCalledWith(["/a"]));
	window.dispatchEvent(
		new PageTransitionEvent("pagehide", { persisted: true }),
	);
	window.dispatchEvent(
		new PageTransitionEvent("pageshow", { persisted: true }),
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
		timeout: 3000,
	});
	fixture.streams[1].send(
		repositoryPaths(fixture.subscriptionId("repository-paths"), 1, ["/b"]),
	);
	await vi.waitFor(() => expect(receive).toHaveBeenLastCalledWith(["/b"]));
});

it("一時的な接続失敗中にページが復帰したら接続をやり直す", async () => {
	const { getConnectionState, onConnectionStateChange } = await import(
		"./client"
	);
	const fixture = stateFixture();
	subscribeState("repository-paths", vi.fn(), vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	const phases: string[] = [];
	const release = onConnectionStateChange(() =>
		phases.push(getConnectionState()),
	);
	fixture.streams[0].fail();
	await vi.waitFor(() =>
		expect(getConnectionState()).toBe("TRANSIENT_FAILURE"),
	);
	window.dispatchEvent(
		new PageTransitionEvent("pagehide", { persisted: true }),
	);
	expect(getConnectionState()).toBe("TRANSIENT_FAILURE");
	window.dispatchEvent(
		new PageTransitionEvent("pageshow", { persisted: true }),
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
		timeout: 3000,
	});
	expect(phases).toEqual(["TRANSIENT_FAILURE", "CONNECTING", "READY"]);
	release();
});

it.each([Code.Unavailable, Code.Aborted, Code.ResourceExhausted])(
	"状態のstreamがcode %sで切れたら最後に受け取った版から購読を再開する",
	async (code) => {
		const fixture = stateFixture();
		const receive = vi.fn();
		subscribeState("repository-paths", receive, vi.fn());
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
		fixture.streams[0].send(
			repositoryPaths(
				fixture.subscriptionId("repository-paths"),
				0,
				["/a"],
				"snapshot",
			),
		);
		fixture.streams[0].send(
			repositoryPaths(fixture.subscriptionId("repository-paths"), 2, ["/b"]),
		);
		await vi.waitFor(() => expect(receive).toHaveBeenCalledWith(["/b"]));
		fixture.streams[0].fail(code);
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
			timeout: 3000,
		});
		expect(fixture.starts[1].version).toMatchObject({
			epoch: "boot",
			sequence: 2n,
		});
		fixture.streams[1].send(
			repositoryPaths(fixture.subscriptionId("repository-paths"), 3, ["/c"]),
		);
		await vi.waitFor(() => expect(receive).toHaveBeenLastCalledWith(["/c"]));
	},
);

it("stream切断から接続を作り直し状態の変化を通知する", async () => {
	const { getConnectionState, onConnectionStateChange } = await import(
		"./client"
	);
	const fixture = stateFixture();
	subscribeState("repository-paths", vi.fn(), vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	const phases: string[] = [];
	const off = onConnectionStateChange(() => phases.push(getConnectionState()));
	fixture.streams[0].fail();
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
		timeout: 3000,
	});
	expect(phases).toEqual(["TRANSIENT_FAILURE", "CONNECTING", "READY"]);
	expect(fixture.serverInfoRequests()).toBe(2);
	off();
});

it("terminal購読をつなぎ直すたびに入力IDを更新する", async () => {
	const { currentTerminalInputId } = await import("./client");
	const fixture = stateFixture();
	const owner = { kind: "workspace" as const, workspacePath: "/repo" };
	subscribeState(
		{ kind: "terminal", args: [owner.workspacePath] },
		vi.fn(),
		vi.fn(),
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	const initial = fixture.starts[0].subscriptionId;
	expect(initial).toBeTruthy();
	expect(currentTerminalInputId(owner)).toBe(initial);
	fixture.streams[0].fail();
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
		timeout: 3000,
	});
	expect(fixture.starts[1].subscriptionId).toBeTruthy();
	expect(fixture.starts[1].subscriptionId).not.toBe(initial);
	expect(currentTerminalInputId(owner)).toBe(fixture.starts[1].subscriptionId);
});

it("terminal入力IDは購読開始の受理後に公開する", async () => {
	let releaseStart: (() => void) | undefined;
	const fixture = stateFixture(
		() =>
			new Promise((resolve) => {
				releaseStart = () => resolve({});
			}),
	);
	const { currentTerminalInputId } = await import("./client");
	const owner = { kind: "workspace" as const, workspacePath: "/repo" };
	subscribeState(
		{ kind: "terminal", args: [owner.workspacePath] },
		vi.fn(),
		vi.fn(),
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	expect(currentTerminalInputId(owner)).toBeNull();
	releaseStart?.();
	await vi.waitFor(() =>
		expect(currentTerminalInputId(owner)).toBe(
			fixture.starts[0].subscriptionId,
		),
	);
});

it("状態のstreamが無通信のまま続いたらつなぎ直す", async () => {
	vi.useFakeTimers();
	try {
		const fixture = stateFixture();
		subscribeState("repository-paths", vi.fn(), vi.fn());
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
		await vi.advanceTimersByTimeAsync(19_000);
		expect(fixture.streams[0].signal.aborted).toBe(false);
		fixture.streams[0].send({
			subscriptionId: fixture.subscriptionId("repository-paths"),
			version: { epoch: "boot", sequence: 0n },
			event: { case: "bookmark", value: {} },
		});
		await vi.advanceTimersByTimeAsync(19_000);
		expect(fixture.streams[0].signal.aborted).toBe(false);
		await vi.advanceTimersByTimeAsync(1_000);
		expect(fixture.streams[0].signal.aborted).toBe(true);
		await vi.advanceTimersByTimeAsync(1_000);
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	} finally {
		window.dispatchEvent(new Event("pagehide"));
		await vi.advanceTimersByTimeAsync(1000);
		vi.useRealTimers();
	}
});

it("状態の受け手が全て停止したらstreamを閉じ再購読で開き直す", async () => {
	const fixture = stateFixture();
	const stopFirst = subscribeState("repository-paths", vi.fn(), vi.fn());
	const stopSecond = subscribeState("repository-paths", vi.fn(), vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	stopFirst();
	expect(fixture.streams[0].signal.aborted).toBe(false);
	stopSecond();
	await vi.waitFor(() => expect(fixture.streams[0].signal.aborted).toBe(true));
	expect(fixture.stops).not.toHaveBeenCalled();
	const receive = vi.fn();
	subscribeState("repository-paths", receive, vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	expect(fixture.starts[1].version).toBeUndefined();
	expect(fixture.streams).toHaveLength(2);
});

it("対象名と引数を別々に送り同じstreamで型付きの状態を届ける", async () => {
	const fixture = stateFixture();
	const received = vi.fn();
	const other = vi.fn();
	const release = subscribeState(
		{ kind: "current-branch", args: ["/作業:repo"] },
		received,
		vi.fn(),
	);
	subscribeState({ kind: "providers", args: [] }, other, vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	expect(fixture.streams).toHaveLength(1);
	expect(fixture.starts[0].target).toBe("current-branch");
	expect(fixture.starts[0].args).toEqual(["/作業:repo"]);
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("current-branch"),
		version: { epoch: "boot", sequence: 1n },
		event: {
			case: "snapshot",
			value: { value: { case: "currentBranch", value: { value: "main" } } },
		},
	});
	await vi.waitFor(() => expect(received).toHaveBeenCalledWith("main"));
	expect(other).not.toHaveBeenCalled();
	release();
	await vi.waitFor(() =>
		expect(fixture.stops).toHaveBeenCalledWith(
			expect.objectContaining({
				subscriptionId: fixture.starts[0].subscriptionId,
			}),
			expect.anything(),
		),
	);
});

it("解除した購読の遅延失敗を同じ対象の新しい購読へ渡さない", async () => {
	let rejectStart: (error: unknown) => void = () => {};
	const start = vi
		.fn()
		.mockImplementationOnce(
			() =>
				new Promise<Record<string, never>>((_, reject) => {
					rejectStart = reject;
				}),
		)
		.mockResolvedValue({});
	const fixture = stateFixture(start);
	const release = subscribeState("repository-paths", vi.fn(), vi.fn());
	subscribeState("providers", vi.fn(), vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	release();
	const error = vi.fn();
	subscribeState("repository-paths", vi.fn(), error);
	rejectStart(new ConnectError("old start failed", Code.Unavailable));
	await vi.waitFor(() => expect(start).toHaveBeenCalledTimes(3));
	await new Promise((resolve) => setTimeout(resolve, 0));
	expect(error).not.toHaveBeenCalled();
});

it("firstStateは最初の値だけで解決して購読を解放する", async () => {
	const fixture = stateFixture();
	subscribeState("providers", vi.fn(), vi.fn());
	const received = vi.fn();
	const result = firstState("repository-paths").then(received);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	fixture.streams[0].send(
		repositoryPaths(
			fixture.subscriptionId("repository-paths"),
			0,
			["/first"],
			"snapshot",
		),
	);
	fixture.streams[0].send(
		repositoryPaths(fixture.subscriptionId("repository-paths"), 1, ["/second"]),
	);
	await result;
	expect(received).toHaveBeenCalledExactlyOnceWith(["/first"]);
	await vi.waitFor(() => expect(fixture.stops).toHaveBeenCalledOnce());
	expect(fixture.stops.mock.calls[0][0]).toMatchObject({
		subscriptionId: fixture.subscriptionId("repository-paths"),
	});
});

it("firstStateは開始失敗を伝播して購読を解放する", async () => {
	const fixture = stateFixture(async () => {
		throw new ConnectError("missing", Code.NotFound);
	});
	const result = firstState("repository-paths");
	await expect(result).rejects.toMatchObject({ code: Code.NotFound });
	await vi.waitFor(() => expect(fixture.streams[0].signal.aborted).toBe(true));
});

it("firstStateは共有済みsnapshotの同期通知でも他の購読を残す", async () => {
	const fixture = stateFixture();
	const release = subscribeState("repository-paths", vi.fn(), vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send(
		repositoryPaths(
			fixture.subscriptionId("repository-paths"),
			0,
			["/cached"],
			"snapshot",
		),
	);
	await vi.waitFor(async () =>
		expect(await firstState("repository-paths")).toEqual(["/cached"]),
	);
	expect(fixture.stops).not.toHaveBeenCalled();
	release();
	await vi.waitFor(() => expect(fixture.streams[0].signal.aborted).toBe(true));
});

it("terminalのsnapshotと差分を他の対象と同じstreamで受け取り最後の出力番号から再開する", async () => {
	const { subscribeTerminalState, reportTerminalProcessed } = await import(
		"./client"
	);
	const fixture = stateFixture();
	const other = vi.fn();
	const stopOther = subscribeState("repository-paths", other, vi.fn());
	const received = vi.fn();
	const pending = subscribeTerminalState(
		{
			owner: { kind: "workspace", workspacePath: "/repo" },
		},
		received,
		vi.fn(),
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	const version = { epoch: "runtime-1", sequence: 17n };
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("terminal"),
		version,
		event: {
			case: "snapshot",
			value: {
				value: {
					case: "terminal",
					value: {
						item: {
							case: "snapshot",
							value: {
								sessionKey: "key",
								sequence: 17n,
								replay: "screen",
								cols: 80,
								rows: 24,
								processedReportUnits: 5000,
							},
						},
					},
				},
			},
		},
	});
	const release = await pending;
	expect(received).toHaveBeenCalledWith(
		expect.objectContaining({
			type: "snapshot",
			surface: expect.objectContaining({ processed_report_units: 5000 }),
		}),
	);
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("terminal"),
		version: { ...version, sequence: 18n },
		event: {
			case: "change",
			value: {
				delta: true,
				payload: {
					value: {
						case: "terminal",
						value: {
							item: {
								case: "output",
								value: { sessionKey: "key", sequence: 18n, data: "next" },
							},
						},
					},
				},
			},
		},
	});
	await vi.waitFor(() =>
		expect(received).toHaveBeenLastCalledWith({
			type: "output",
			session_key: "key",
			sequence: 18,
			data: "next",
		}),
	);
	expect(fixture.streams).toHaveLength(1);
	await reportTerminalProcessed(
		{ kind: "workspace", workspacePath: "/repo" },
		5000,
	);
	expect(fixture.reports).toHaveBeenCalledOnce();
	expect(fixture.reports.mock.calls[0][0]).toMatchObject({
		subscriptionId: fixture.subscriptionId("terminal"),
		units: 5000,
	});
	expect(fixture.reports.mock.calls[0][0]).not.toHaveProperty("clientId");
	expect(fixture.reports.mock.calls[0][0]).not.toHaveProperty("args");
	fixture.streams[0].fail();
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(4), {
		timeout: 3000,
	});
	expect(
		fixture.starts.filter((start) => start.target === "terminal")[1],
	).toMatchObject({
		version: { ...version, sequence: 18n },
		subscriptionId: expect.any(String),
	});
	await release();
	stopOther();
});

it("読み取り失敗と最後の値を後続購読者へ渡し回復後に失敗を解除する", async () => {
	const fixture = stateFixture();
	const value = vi.fn();
	const error = vi.fn();
	const release = subscribeState("repository-paths", value, error);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("repository-paths"),
		event: {
			case: "snapshot",
			value: {
				value: { case: "repositoryPaths", value: { items: ["/repo"] } },
			},
		},
	});
	await vi.waitFor(() => expect(value).toHaveBeenCalledWith(["/repo"]));
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("repository-paths"),
		event: {
			case: "failure",
			value: { code: Code.Internal, message: "read failed" },
		},
	});
	await vi.waitFor(() => expect(error).toHaveBeenCalledOnce());
	const laterValue = vi.fn();
	const laterError = vi.fn();
	const laterRelease = subscribeState(
		"repository-paths",
		laterValue,
		laterError,
	);
	expect(laterError).toHaveBeenCalledWith(
		expect.objectContaining({ code: Code.Internal }),
	);
	expect(laterValue).toHaveBeenCalledWith(["/repo"]);
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("repository-paths"),
		event: {
			case: "snapshot",
			value: {
				value: { case: "repositoryPaths", value: { items: ["/recovered"] } },
			},
		},
	});
	await vi.waitFor(() =>
		expect(laterValue).toHaveBeenCalledWith(["/recovered"]),
	);
	const recoveredError = vi.fn();
	const recoveredRelease = subscribeState(
		"repository-paths",
		vi.fn(),
		recoveredError,
	);
	expect(recoveredError).not.toHaveBeenCalled();
	recoveredRelease();
	laterRelease();
	release();
});

it("端末の後続購読は過去の読取失敗を受けず新しい読取を待つ", async () => {
	const fixture = stateFixture();
	const target = { kind: "terminal" as const, args: ["/repo"] };
	const error = vi.fn();
	const release = subscribeState(target, vi.fn(), error);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("terminal"),
		event: {
			case: "failure",
			value: { code: Code.Internal, message: "terminal read failed" },
		},
	});
	await vi.waitFor(() => expect(error).toHaveBeenCalledOnce());
	const laterError = vi.fn();
	const laterRelease = subscribeState(target, vi.fn(), laterError);
	expect(laterError).not.toHaveBeenCalled();
	laterRelease();
	release();
});

it("terminal再購読は過去の失敗では終了せず新しい値で解決する", async () => {
	const { subscribeTerminalState } = await import("./client");
	const fixture = stateFixture();
	const target = { kind: "terminal" as const, args: ["/repo"] };
	const failed = vi.fn();
	const stop = subscribeState(target, vi.fn(), failed);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("terminal"),
		event: {
			case: "failure",
			value: { code: Code.Internal, message: "old failure" },
		},
	});
	await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
	const pending = subscribeTerminalState(
		{ owner: { kind: "workspace", workspacePath: "/repo" } },
		vi.fn(),
		vi.fn(),
	);
	let settled = false;
	void pending.then(
		() => {
			settled = true;
		},
		() => {
			settled = true;
		},
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	expect(settled).toBe(false);
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("terminal"),
		event: {
			case: "snapshot",
			value: {
				value: {
					case: "terminal",
					value: {
						item: {
							case: "snapshot",
							value: {
								sessionKey: "key",
								sequence: 1n,
								replay: "screen",
								cols: 80,
								rows: 24,
								processedReportUnits: 5000,
							},
						},
					},
				},
			},
		},
	});
	const release = await pending;
	await release();
	stop();
});

it("terminal再購読は過去の失敗では終了せず新しい失敗で拒否する", async () => {
	const { subscribeTerminalState } = await import("./client");
	const fixture = stateFixture();
	const target = { kind: "terminal" as const, args: ["/repo"] };
	const failed = vi.fn();
	const stop = subscribeState(target, vi.fn(), failed);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("terminal"),
		event: {
			case: "failure",
			value: { code: Code.Internal, message: "old failure" },
		},
	});
	await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
	const pending = subscribeTerminalState(
		{ owner: { kind: "workspace", workspacePath: "/repo" } },
		vi.fn(),
		vi.fn(),
	);
	let settled = false;
	void pending.then(
		() => {
			settled = true;
		},
		() => {
			settled = true;
		},
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	expect(settled).toBe(false);
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("terminal"),
		event: {
			case: "failure",
			value: { code: Code.Internal, message: "new failure" },
		},
	});
	await expect(pending).rejects.toMatchObject({ rawMessage: "new failure" });
	stop();
});

it("つなぎ直しの購読開始が失敗しても最後の値と失敗を後続購読者へ渡す", async () => {
	const start = vi
		.fn()
		.mockResolvedValueOnce({})
		.mockRejectedValueOnce(new ConnectError("start denied", Code.NotFound));
	const fixture = stateFixture(start);
	const value = vi.fn();
	const error = vi.fn();
	const release = subscribeState("repository-paths", value, error);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send(
		repositoryPaths(
			fixture.subscriptionId("repository-paths"),
			1,
			["/repo"],
			"snapshot",
		),
	);
	await vi.waitFor(() => expect(value).toHaveBeenCalledWith(["/repo"]));
	fixture.streams[0].fail();
	await vi.waitFor(() => expect(error).toHaveBeenCalledOnce(), {
		timeout: 3000,
	});
	const laterValue = vi.fn();
	const laterError = vi.fn();
	const laterRelease = subscribeState(
		"repository-paths",
		laterValue,
		laterError,
	);
	expect(laterValue).toHaveBeenCalledWith(["/repo"]);
	expect(laterError).toHaveBeenCalledWith(
		expect.objectContaining({ code: Code.NotFound }),
	);
	laterRelease();
	release();
});

it("PR未取得の行を含むWorkspacesを実際のpayloadから復号して受け手へ届ける", async () => {
	const fixture = stateFixture();
	const receive = vi.fn();
	const fail = vi.fn();
	const release = subscribeState("workspaces", receive, fail);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	const branch = {
		name: "feature",
		isMainWorktree: false,
		isDeleting: false,
		worktreePath: "/repo/feature",
		isMerged: false,
		pullRequestError: "PR denied",
	};
	fixture.streams[0].send({
		subscriptionId: fixture.subscriptionId("workspaces"),
		version: { epoch: "boot", sequence: 0n },
		event: {
			case: "snapshot",
			value: {
				value: {
					case: "workspaces",
					value: {
						status: { state: "ready", loaded: true },
						repositories: {
							items: [
								{
									path: "/repo",
									status: { state: "ready", loaded: true },
									branches: {
										items: [
											branch,
											{
												...branch,
												hasPr: true,
												prNumber: 42n,
												prUrl: "https://example.test/pr/42",
											},
										],
									},
									worktrees: { items: [] },
								},
							],
						},
					},
				},
			},
		},
	});
	await vi.waitFor(() => expect(receive).toHaveBeenCalledOnce());
	expect(receive.mock.calls[0][0].repositories[0].branches).toEqual([
		expect.objectContaining({
			has_pr: null,
			pr_number: null,
			pr_url: null,
			pull_request_error: "PR denied",
		}),
		expect.objectContaining({
			has_pr: true,
			pr_number: 42,
			pr_url: "https://example.test/pr/42",
			pull_request_error: "PR denied",
		}),
	]);
	expect(fail).not.toHaveBeenCalled();
	release();
});

it("Notionの入力をそのまま送り要求元の引数で値と失敗を対応付ける", async () => {
	const fixture = stateFixture();
	const keepStream = subscribeState("repository-paths", vi.fn(), vi.fn());
	const a = vi.fn();
	const b = vi.fn();
	const releaseA = subscribeState(
		{
			kind: "notion-tasks",
			args: ["/repo", "20", 'labels={"Tags":["z","a"],"Status":["Todo"]}'],
		},
		a,
		vi.fn(),
	);
	const releaseB = subscribeState(
		{
			kind: "notion-tasks",
			args: ["/repo", "20", 'labels={"Status":["Todo"],"Tags":["a","z"]}'],
		},
		b,
		vi.fn(),
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(3));
	for (const request of fixture.starts.slice(1)) {
		fixture.streams[0].send({
			subscriptionId: request.subscriptionId,
			version: { epoch: "boot", sequence: 1n },
			event: {
				case: "snapshot",
				value: {
					value: {
						case: "notionTasks",
						value: {
							page: { tasks: { items: [] }, hasMore: true },
							readError: {
								code: 9,
								message: "not configured",
								configMissing: true,
							},
						},
					},
				},
			},
		});
	}
	await vi.waitFor(() =>
		expect(a).toHaveBeenCalledWith({
			page: { tasks: [], has_more: true },
			readError: { code: 9, message: "not configured", configMissing: true },
		}),
	);
	expect(b).toHaveBeenCalledWith({
		page: { tasks: [], has_more: true },
		readError: { code: 9, message: "not configured", configMissing: true },
	});
	expect(fixture.starts.slice(1).map((request) => request.args)).toEqual([
		["/repo", "20", 'labels={"Tags":["z","a"],"Status":["Todo"]}'],
		["/repo", "20", 'labels={"Status":["Todo"],"Tags":["a","z"]}'],
	]);
	releaseA();
	await vi.waitFor(() => expect(fixture.stops).toHaveBeenCalledOnce());
	releaseB();
	await vi.waitFor(() => expect(fixture.stops).toHaveBeenCalledTimes(2));
	keepStream();
});

it("不正なラベル引数はキー生成で例外にならずdaemonの検証失敗を受け取る", async () => {
	const fixture = stateFixture(async () => {
		throw new ConnectError("Invalid labels", Code.InvalidArgument);
	});
	const fail = vi.fn();
	const release = subscribeState(
		{ kind: "notion-tasks", args: ["/repo", "20", "labels=invalid"] },
		vi.fn(),
		fail,
	);
	await vi.waitFor(() =>
		expect(fail).toHaveBeenCalledWith(
			expect.objectContaining({ code: Code.InvalidArgument }),
		),
	);
	expect(fixture.starts[0].args).toEqual(["/repo", "20", "labels=invalid"]);
	release();
});

it("停止は購読識別子だけを送り古い識別子の値を渡さない", async () => {
	const fixture = stateFixture();
	const received = vi.fn();
	const release = subscribeState("repository-paths", received, vi.fn());
	subscribeState("providers", vi.fn(), vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	const oldId = fixture.subscriptionId("repository-paths");
	release();
	subscribeState("repository-paths", received, vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(3));
	const newId = fixture.subscriptionId("repository-paths");
	expect(newId).not.toBe(oldId);
	fixture.streams[0].send(repositoryPaths(oldId, 1, ["/old"]));
	fixture.streams[0].send(repositoryPaths(newId, 1, ["/new"]));
	await vi.waitFor(() =>
		expect(received).toHaveBeenCalledExactlyOnceWith(["/new"]),
	);
	expect(fixture.stops.mock.calls[0][0]).toEqual(
		expect.objectContaining({ subscriptionId: oldId }),
	);
	expect(fixture.stops.mock.calls[0][0]).not.toHaveProperty("target");
	expect(fixture.stops.mock.calls[0][0]).not.toHaveProperty("args");
	expect(fixture.stops.mock.calls[0][0]).not.toHaveProperty("clientId");
});

it.each([true, false])(
	"開始中の解除は同じIDの開始完了後に停止する（別購読あり=%s）",
	async (keepStream) => {
		let completeStart: (() => void) | undefined;
		const active = new Set<string>();
		const fixture = stateFixture(
			async (request) => {
				active.add(request.subscriptionId);
				if (request.target === "repository-paths") {
					await new Promise<void>((resolve) => {
						completeStart = resolve;
					});
				}
				return {};
			},
			{
				stop: (request) => {
					active.delete(request.subscriptionId);
					return {};
				},
			},
		);
		const release = subscribeState("repository-paths", vi.fn(), vi.fn());
		if (keepStream) subscribeState("providers", vi.fn(), vi.fn());
		await vi.waitFor(() =>
			expect(fixture.starts).toHaveLength(keepStream ? 2 : 1),
		);
		const id = fixture.subscriptionId("repository-paths");
		release();
		await new Promise((resolve) => setTimeout(resolve, 0));
		expect(fixture.stops).not.toHaveBeenCalled();
		expect(active.has(id)).toBe(true);
		expect(fixture.streams[0].signal.aborted).toBe(!keepStream);
		completeStart?.();
		await vi.waitFor(() =>
			expect(fixture.stops).toHaveBeenCalledExactlyOnceWith(
				expect.objectContaining({ subscriptionId: id }),
				expect.anything(),
			),
		);
		expect(active.has(id)).toBe(false);
		expect(active.size).toBe(keepStream ? 1 : 0);
	},
);

it("terminalの旧開始が保留中でも再開始後の入力先が失効しない", async () => {
	let completeFirst: (() => void) | undefined;
	let input: string | null = null;
	const accepted: string[] = [];
	const fixture = stateFixture(
		async (request) => {
			if (!completeFirst)
				await new Promise<void>((resolve) => {
					completeFirst = resolve;
				});
			input = request.subscriptionId;
			return {};
		},
		{
			writeTerminalSurface: (request) => {
				if (!request.attachmentId || request.attachmentId !== input)
					throw new ConnectError(
						"Stale terminal input",
						Code.FailedPrecondition,
					);
				accepted.push(request.attachmentId);
				return {};
			},
			stop: (request) => {
				if (input === request.subscriptionId) input = null;
				return {};
			},
		},
	);
	const { currentTerminalInputId } = await import("./client");
	const owner = { kind: "workspace" as const, workspacePath: "/repo" };
	const target = { kind: "terminal" as const, args: [owner.workspacePath] };
	const releaseFirst = subscribeState(target, vi.fn(), vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	const first = fixture.starts[0].subscriptionId;
	const releaseSecond = subscribeState(target, vi.fn(), vi.fn());
	await new Promise((resolve) => setTimeout(resolve, 0));
	expect(fixture.starts).toHaveLength(1);
	expect(fixture.stops).not.toHaveBeenCalled();
	expect(currentTerminalInputId(owner)).toBeNull();
	completeFirst?.();
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	const second = fixture.starts[1].subscriptionId;
	await vi.waitFor(() =>
		expect(fixture.stops).toHaveBeenCalledWith(
			expect.objectContaining({ subscriptionId: first }),
			expect.anything(),
		),
	);
	await vi.waitFor(() => expect(currentTerminalInputId(owner)).toBe(second));
	const destination = currentTerminalInputId(owner);
	if (!destination)
		throw new Error("Terminal input destination is unavailable");
	await invokeClient("write_terminal_surface", {
		owner,
		attachmentId: destination,
		sequence: 0,
		data: "x",
	});
	expect(accepted).toEqual([second]);
	expect(input).toBe(second);
	expect(second).not.toBe(first);
	releaseFirst();
	releaseSecond();
});
