import type { MessageInitShape } from "@bufbuild/protobuf";
import { Code, ConnectError, type HandlerContext } from "@connectrpc/connect";
import { afterEach, expect, it, vi } from "vitest";
import {
	CommandErrorSchema,
	type StartStateSubscriptionRequest,
	type StateSubscriptionEventSchema,
} from "@/generated/client_pb";
import { connectFixture } from "@/test/connect";
import { firstState, invokeClient, subscribeState } from "./client";
import { getErrorMessage } from "./errorMessage";

vi.unmock("@/lib/client");
const requestUrl = (input: RequestInfo | URL) =>
	input instanceof Request ? input.url : input.toString();
afterEach(async () => {
	window.dispatchEvent(new Event("pagehide"));
	await new Promise((resolve) => setTimeout(resolve, 0));
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

it("旧世代の要求失敗が新接続と進行中の要求を破棄しない", async () => {
	const { refreshClient, getClient } = await import("./client");
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
	refreshClient();
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
	const { getClient, refreshClient } = await import("./client");
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
	const release = subscribeState("repository-paths", vi.fn());
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
	refreshClient();
	expect(streaming?.signal.aborted).toBe(true);
	release();
});

type StateEvent = MessageInitShape<typeof StateSubscriptionEventSchema>;

function stateFixture(
	start?: () => Promise<Record<string, never>>,
	options: {
		failBeforeReady?: number;
		stop?: () => Record<string, never>;
	} = {},
) {
	const streams: {
		send: (event: StateEvent) => void;
		fail: (code?: Code) => void;
		signal: AbortSignal;
		openedAt: number;
	}[] = [];
	const starts: StartStateSubscriptionRequest[] = [];
	const reports = vi.fn(() => ({}));
	const stops = vi.fn((_request: { target: string }) => options.stop?.() ?? {});
	const fixture = connectFixture({
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
			return start?.() ?? {};
		},
		stopStateSubscription: stops,
		reportTerminalProcessed: reports,
	});
	const serverInfoRequests = () =>
		fixture.requests.filter((request) => request.url.endsWith("/GetServerInfo"))
			.length;
	return { streams, starts, stops, reports, serverInfoRequests };
}

it("状態のstreamがreadyの直後に切れ続けても待ちが伸びる", async () => {
	vi.useFakeTimers();
	vi.spyOn(Math, "random").mockReturnValue(0.5);
	try {
		const fixture = stateFixture();
		subscribeState("repository-paths", vi.fn());
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
		subscribeState("repository-paths", vi.fn());
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
		subscribeState("repository-paths", vi.fn());
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
		subscribeState("repository-paths", vi.fn());
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
		fixture.streams[0].fail();
		await vi.advanceTimersByTimeAsync(1000);
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
		for (let i = 0; i < 6; i++) {
			await vi.advanceTimersByTimeAsync(20000);
			fixture.streams[1].send({
				target: "repository-paths",
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
	"購読の開始がcode %sで失敗したら接続を使い回してstreamを開き直す",
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

it.each([Code.Unavailable, Code.Aborted, Code.ResourceExhausted])(
	"購読の停止がcode %sで失敗したらstreamを開き直し残った対象だけを開始する",
	async (code) => {
		const fixture = stateFixture(undefined, {
			stop: () => {
				throw new ConnectError("state lost", code);
			},
		});
		const release = subscribeState("repository-paths", vi.fn());
		subscribeState("providers", vi.fn());
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
	sequence: number,
	items: string[],
	kind: "snapshot" | "change" = "change",
): StateEvent => ({
	target: "repository-paths",
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
	subscribeState("repository-paths", first);
	subscribeState("repository-paths", second);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	expect(fixture.starts[0].target).toBe("repository-paths");
	expect(fixture.starts[0].version).toBeUndefined();
	fixture.streams[0].send(repositoryPaths(0, ["/a"], "snapshot"));
	fixture.streams[0].send({
		target: "repository-paths",
		version: { epoch: "boot", sequence: 0n },
		event: { case: "bookmark", value: {} },
	});
	fixture.streams[0].send(repositoryPaths(1, ["/a", "/b"]));
	await vi.waitFor(() => {
		expect(first.mock.calls).toEqual([[["/a"]], [["/a", "/b"]]]);
		expect(second.mock.calls).toEqual([[["/a"]], [["/a", "/b"]]]);
	});
	const late = vi.fn();
	subscribeState("repository-paths", late);
	expect(late).toHaveBeenCalledWith(["/a", "/b"]);
	expect(fixture.starts).toHaveLength(1);
});

it.each([Code.Unavailable, Code.Aborted, Code.ResourceExhausted])(
	"状態のstreamがcode %sで切れたら最後に受け取った版から購読を再開する",
	async (code) => {
		const fixture = stateFixture();
		const receive = vi.fn();
		subscribeState("repository-paths", receive);
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
		fixture.streams[0].send(repositoryPaths(0, ["/a"], "snapshot"));
		fixture.streams[0].send(repositoryPaths(2, ["/b"]));
		await vi.waitFor(() => expect(receive).toHaveBeenCalledWith(["/b"]));
		fixture.streams[0].fail(code);
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
			timeout: 3000,
		});
		expect(fixture.starts[1].version).toMatchObject({
			epoch: "boot",
			sequence: 2n,
		});
		fixture.streams[1].send(repositoryPaths(3, ["/c"]));
		await vi.waitFor(() => expect(receive).toHaveBeenLastCalledWith(["/c"]));
	},
);

it("接続を作り直すと状態のstreamのつなぎ直しで接続の回復を通知する", async () => {
	const { onClientConnection, refreshClient } = await import("./client");
	const fixture = stateFixture();
	subscribeState("repository-paths", vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	const connection = vi.fn();
	const off = onClientConnection(connection);
	refreshClient();
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
		timeout: 3000,
	});
	expect(connection.mock.calls.map(([connected]) => connected)).toEqual([
		false,
		true,
	]);
	expect(fixture.serverInfoRequests()).toBe(2);
	off();
});

it("状態のstreamが無通信のまま続いたらつなぎ直す", async () => {
	vi.useFakeTimers();
	try {
		const fixture = stateFixture();
		subscribeState("repository-paths", vi.fn());
		await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
		await vi.advanceTimersByTimeAsync(29_000);
		expect(fixture.streams[0].signal.aborted).toBe(false);
		fixture.streams[0].send({
			target: "repository-paths",
			version: { epoch: "boot", sequence: 0n },
			event: { case: "bookmark", value: {} },
		});
		await vi.advanceTimersByTimeAsync(29_000);
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
	const stopFirst = subscribeState("repository-paths", vi.fn());
	const stopSecond = subscribeState("repository-paths", vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	stopFirst();
	expect(fixture.streams[0].signal.aborted).toBe(false);
	stopSecond();
	await vi.waitFor(() => expect(fixture.streams[0].signal.aborted).toBe(true));
	expect(fixture.stops).not.toHaveBeenCalled();
	const receive = vi.fn();
	subscribeState("repository-paths", receive);
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
	);
	subscribeState({ kind: "providers", args: [] }, other);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	expect(fixture.streams).toHaveLength(1);
	expect(fixture.starts[0].target).toBe("current-branch");
	expect(fixture.starts[0].args).toEqual(["/作業:repo"]);
	fixture.streams[0].send({
		target: "current-branch",
		args: ["/作業:repo"],
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
				target: "current-branch",
				args: ["/作業:repo"],
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
	const release = subscribeState("repository-paths", vi.fn());
	subscribeState("providers", vi.fn());
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
	subscribeState("providers", vi.fn());
	const received = vi.fn();
	const result = firstState("repository-paths").then(received);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	fixture.streams[0].send(repositoryPaths(0, ["/first"], "snapshot"));
	fixture.streams[0].send(repositoryPaths(1, ["/second"]));
	await result;
	expect(received).toHaveBeenCalledExactlyOnceWith(["/first"]);
	await vi.waitFor(() => expect(fixture.stops).toHaveBeenCalledOnce());
	expect(fixture.stops.mock.calls[0][0]).toMatchObject({
		target: "repository-paths",
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
	const release = subscribeState("repository-paths", vi.fn());
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send(repositoryPaths(0, ["/cached"], "snapshot"));
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
	const stopOther = subscribeState("repository-paths", other);
	const received = vi.fn();
	const pending = subscribeTerminalState(
		{
			owner: { kind: "workspace", workspacePath: "/repo" },
			attachmentId: "input",
		},
		received,
		vi.fn(),
	);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2));
	const version = { epoch: "runtime-1", sequence: 17n };
	fixture.streams[0].send({
		target: "terminal",
		args: ["/repo"],
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
		target: "terminal",
		args: ["/repo"],
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
	fixture.streams[0].fail();
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(4), {
		timeout: 3000,
	});
	expect(
		fixture.starts.filter((start) => start.target === "terminal")[1],
	).toMatchObject({
		version: { ...version, sequence: 18n },
		terminalInputId: "input",
	});
	await release();
	stopOther();
});
