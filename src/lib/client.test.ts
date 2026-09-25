import { create, type MessageInitShape } from "@bufbuild/protobuf";
import { Code, ConnectError, type HandlerContext } from "@connectrpc/connect";
import { afterEach, expect, it, vi } from "vitest";
import {
	CommandErrorSchema,
	PushSchema,
	type StartStateSubscriptionRequest,
	type StateSubscriptionEventSchema,
} from "@/generated/client_pb";
import { connectFixture } from "@/test/connect";
import {
	firstState,
	invokeClient,
	onClientRefresh,
	subscribeState,
} from "./client";
import * as clientProtocol from "./clientProtocol";
import { getErrorMessage } from "./errorMessage";

vi.unmock("@/lib/client");
afterEach(async () => {
	window.dispatchEvent(new Event("pagehide"));
	await new Promise((resolve) => setTimeout(resolve, 0));
	vi.unstubAllGlobals();
});

it("生成RPCで引数と結果を変換する", async () => {
	const read = vi.fn(() => ({ value: "/repo" }));
	const fixture = connectFixture({ getExternalEditor: read });
	await expect(invokeClient("get_external_editor")).resolves.toEqual("/repo");
	expect(read).toHaveBeenCalledOnce();
	expect(
		fixture.requests.map((request) => new URL(request.url).pathname),
	).toEqual([
		"/releash.client.v1.ClientService/GetServerInfo",
		"/releash.client.v1.ClientService/GetExternalEditor",
	]);
});

it("応答未到達の変更要求では再接続せず明示的な再接続後に現在状態だけを再取得する", async () => {
	const { getClient, refreshClient } = await import("./client");
	let value = false;
	const mutate = vi.fn(() => {
		value = true;
		throw new ConnectError("response lost", Code.Unavailable);
	});
	const fixture = connectFixture({
		updateCrashReporting: mutate,
		getPerformanceTelemetryEnabled: () => ({ value }),
	});
	const observed: boolean[] = [];
	const refresh = vi.fn(() => {
		void invokeClient("get_performance_telemetry_enabled").then((value) =>
			observed.push(value),
		);
	});
	const stop = onClientRefresh(refresh);
	await vi.waitFor(() => expect(observed).toContain(false));
	const client = await getClient();
	await expect(
		invokeClient("update_crash_reporting", { enabled: true }),
	).rejects.toBeInstanceOf(ConnectError);
	expect(await getClient()).toBe(client);
	expect(observed).toEqual([false]);
	refreshClient();
	await vi.waitFor(() => expect(observed).toContain(true), { timeout: 3000 });
	expect(mutate).toHaveBeenCalledOnce();
	expect(
		fixture.requests.every(
			(request) => !/Operation|Query|AckRequest/.test(request.url),
		),
	).toBe(true);
	stop();
});

it("送信失敗のPromiseを終了し元要求を保持して再送しない", async () => {
	const fixture = connectFixture();
	const original = fixture.fetch.getMockImplementation();
	if (!original) throw new Error("Missing fetch fixture");
	let writes = 0;
	fixture.fetch.mockImplementation(async (input, init) => {
		const request = new Request(input, init);
		if (request.url.endsWith("/UpdateExternalEditor")) {
			writes++;
			throw new TypeError("network failure");
		}
		return original(request);
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

it("監視はpush一本を共有し切断後に再登録して解除時に停止する", async () => {
	const { watchClient, refreshClient } = await import("./client");
	let attempts = 0;
	const stopWatching = vi.fn((_request: { watcherId?: bigint }) => ({}));
	const fixture = connectFixture({
		watchFiles: () => ({ value: BigInt(++attempts) }),
		stopWatching,
	});
	const ready = vi.fn();
	const stop = watchClient({ path: "/repo" }, ready, vi.fn());
	await vi.waitFor(() => expect(ready).toHaveBeenCalledWith(1));
	refreshClient();
	await vi.waitFor(() => expect(ready).toHaveBeenCalledWith(2), {
		timeout: 3000,
	});
	expect(
		fixture.requests.filter((request) =>
			request.url.endsWith("/SubscribePush"),
		),
	).toHaveLength(2);
	stop();
	await vi.waitFor(() => expect(stopWatching).toHaveBeenCalledOnce());
	expect(stopWatching.mock.calls.map((call) => call[0])).toEqual([
		expect.objectContaining({ watcherId: 2n }),
	]);
});

it("監視登録中の解除は応答後に資源を停止する", async () => {
	const { watchClient } = await import("./client");
	let complete!: () => void;
	const registered = vi.fn(async () => {
		await new Promise<void>((resolve) => {
			complete = resolve;
		});
		return { value: 7n };
	});
	const stopped = vi.fn(() => ({}));
	connectFixture({ watchFiles: registered, stopWatching: stopped });
	const ready = vi.fn();
	const stop = watchClient({ path: "/repo" }, ready);
	await vi.waitFor(() => expect(registered).toHaveBeenCalledOnce());
	stop();
	complete();
	await vi.waitFor(() => expect(stopped).toHaveBeenCalledOnce());
	expect(ready).not.toHaveBeenCalled();
});

it.each(["start_watching"] as const)(
	"%sは再購読時の上限超過後も枠が空けば同じ購読で監視を復旧する",
	async () => {
		vi.useFakeTimers();
		const { getClient, refreshClient, watchClient } = await import("./client");
		let available = true;
		const watch = vi.fn((_request: { subscriptionId: string }) => {
			if (!available)
				throw new ConnectError(
					"Too many subscriptions",
					Code.ResourceExhausted,
				);
			return { value: 1n };
		});
		const fixture = connectFixture({
			watchFiles: watch,
			stopWatching: () => ({}),
		});
		const ready = vi.fn();
		const failed = vi.fn();
		const stop = watchClient({ path: "/repo" }, ready, failed);
		try {
			await vi.waitFor(() => expect(ready).toHaveBeenCalledOnce());
			available = false;
			refreshClient();
			await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce(), {
				timeout: 2000,
			});
			const client = await getClient();
			await vi.advanceTimersByTimeAsync(1000);
			expect(watch).toHaveBeenCalledTimes(3);
			available = true;
			await vi.advanceTimersByTimeAsync(1000);
			expect(ready).toHaveBeenCalledTimes(2);
			expect(await getClient()).toBe(client);
			const ids = watch.mock.calls.map(([request]) => request.subscriptionId);
			expect(ids[0]).not.toBe(ids[1]);
			expect(ids.slice(1)).toEqual([ids[1], ids[1], ids[1]]);
			expect(
				fixture.requests.filter((request) =>
					request.url.endsWith("/SubscribePush"),
				),
			).toHaveLength(2);
		} finally {
			stop();
			window.dispatchEvent(new Event("pagehide"));
			await vi.advanceTimersByTimeAsync(1000);
			vi.useRealTimers();
		}
	},
);

it.each(["stop", "pagehide", "reconnect"])(
	"監視登録の再試行待ちで%sした旧購読を再登録しない",
	async (ending) => {
		vi.useFakeTimers();
		const { refreshClient, watchClient } = await import("./client");
		const watch = vi.fn(() => {
			throw new ConnectError("Too many subscriptions", Code.ResourceExhausted);
		});
		connectFixture({ watchFiles: watch });
		const failed = vi.fn();
		const stop = watchClient({ path: "/repo" }, vi.fn(), failed);
		try {
			await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
			if (ending === "stop") stop();
			else if (ending === "pagehide")
				window.dispatchEvent(new Event("pagehide"));
			else refreshClient();
			await vi.advanceTimersByTimeAsync(1000);
			expect(watch).toHaveBeenCalledTimes(ending === "reconnect" ? 2 : 1);
		} finally {
			stop();
			window.dispatchEvent(new Event("pagehide"));
			await vi.advanceTimersByTimeAsync(1000);
			vi.useRealTimers();
		}
	},
);

it("旧世代の要求失敗が新接続と進行中の要求を破棄しない", async () => {
	const { refreshClient, getClient } = await import("./client");
	let release!: () => void;
	const read = vi.fn(async () => {
		await new Promise<void>((resolve) => {
			release = resolve;
		});
		return { value: "/current" };
	});
	const fixture = connectFixture({ getExternalEditor: read });
	const original = fixture.fetch.getMockImplementation();
	if (!original) throw new Error("Missing fixture implementation");
	let fail!: () => void;
	fixture.fetch.mockImplementation(async (input, init) => {
		const request = new Request(input, init);
		if (request.url.endsWith("/UpdateExternalEditor"))
			return new Promise<Response>((_, reject) => {
				fail = () => reject(new TypeError("late failure"));
			});
		return original(request);
	});
	const old = invokeClient("update_external_editor", { editor: "vim" }).catch(
		(error) => error,
	);
	await vi.waitFor(() => expect(fail).toBeDefined());
	refreshClient();
	const current = await getClient();
	const pending = invokeClient("get_external_editor");
	await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
	fail();
	expect(await old).toBeInstanceOf(ConnectError);
	expect(await getClient()).toBe(current);
	await release();
	await expect(pending).resolves.toEqual("/current");
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
		getExternalEditor: () => {
			throw commandError("REPOSITORY_UNAVAILABLE", "Repository is unavailable");
		},
	});
	const client = await getClient();
	const error = await invokeClient("get_external_editor").catch(
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
])("RPC失敗code %sは共有接続と進行中のRPCとpushを中断しない", async (code) => {
	const { getClient, watchClient } = await import("./client");
	let release!: () => void;
	const read = vi.fn(async () => {
		await new Promise<void>((resolve) => {
			release = resolve;
		});
		return { value: "/repo" };
	});
	const reject = () => {
		throw new ConnectError("request rejected", code);
	};
	const fixture = connectFixture({
		getExternalEditor: read,
		updateExternalEditor: reject,
		watchFiles: reject,
	});
	const refreshed = vi.fn();
	onClientRefresh(refreshed);
	await vi.waitFor(() => expect(refreshed).toHaveBeenCalledOnce());
	const client = await getClient();
	const pending = invokeClient("get_external_editor");
	await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
	const error = await invokeClient("update_external_editor", {
		editor: "vim",
	}).catch((error) => error);
	expect(error).toBeInstanceOf(ConnectError);
	expect(error).toMatchObject({ code, rawMessage: "request rejected" });
	{
		const failed = vi.fn();
		const stop = watchClient({ path: "/repo" }, vi.fn(), failed);
		await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
		expect(failed.mock.calls[0][0]).toMatchObject({ code });
		stop();
	}
	expect(await getClient()).toBe(client);
	expect(
		fixture.requests.find((request) =>
			request.url.endsWith("/GetExternalEditor"),
		)?.signal.aborted,
	).toBe(false);
	expect(
		fixture.requests.find((request) => request.url.endsWith("/SubscribePush"))
			?.signal.aborted,
	).toBe(false);
	expect(refreshed).toHaveBeenCalledOnce();
	await release();
	await expect(pending).resolves.toEqual("/repo");
});

it.each(["end", "failure"])(
	"push単独の%sから再購読と状態再取得を行う",
	async (ending) => {
		const { listenClient, watchClient } = await import("./client");
		let finish!: () => void;
		let subscriptions = 0;
		let value = false;
		const watch = vi.fn(() => ({ value: 1n }));
		const fixture = connectFixture({
			getPerformanceTelemetryEnabled: () => ({ value }),
			watchFiles: watch,
			stopWatching: () => ({}),
			async *subscribePush(_, context) {
				const attempt = ++subscriptions;
				yield create(PushSchema, { event: { case: "resync", value: {} } });
				if (attempt === 1) {
					await new Promise<void>((resolve) => {
						finish = resolve;
					});
					if (ending === "failure")
						throw new ConnectError("push lost", Code.Unavailable);
				} else {
					yield create(PushSchema, {
						event: {
							case: "reviewCommentsChanged",
							value: { value: "/recovered" },
						},
					});
					await new Promise<void>((resolve) =>
						context.signal.addEventListener("abort", () => resolve(), {
							once: true,
						}),
					);
				}
			},
		});
		const observed: boolean[] = [];
		onClientRefresh(() => {
			void invokeClient("get_performance_telemetry_enabled").then((value) =>
				observed.push(value),
			);
		});
		const pushed = vi.fn();
		await listenClient("review-comments-changed", pushed);
		const stop = watchClient({ path: "/repo" }, vi.fn());
		await vi.waitFor(() => {
			expect(observed).toEqual([false]);
			expect(watch).toHaveBeenCalledOnce();
			expect(finish).toBeDefined();
		});
		value = true;
		finish();
		await vi.waitFor(
			() => {
				expect(observed).toEqual([false, true]);
				expect(watch).toHaveBeenCalledTimes(2);
				expect(pushed).toHaveBeenCalledWith({ payload: "/recovered" });
			},
			{ timeout: 3000 },
		);
		expect(subscriptions).toBe(2);
		expect(
			fixture.requests.filter((request) =>
				request.url.endsWith("/GetServerInfo"),
			),
		).toHaveLength(2);
		stop();
	},
);

it("同じpushのpayloadは購読者が複数でも一度だけ復号する", async () => {
	const { listenClient } = await import("./client");
	let publish!: () => void;
	connectFixture({
		async *subscribePush(_, context) {
			yield create(PushSchema, { event: { case: "resync", value: {} } });
			await new Promise<void>((resolve) => {
				publish = resolve;
			});
			yield create(PushSchema, {
				event: { case: "reviewCommentsChanged", value: { value: "/repo" } },
			});
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
		},
	});
	const decode = vi.spyOn(clientProtocol, "decodeClientPush");
	try {
		const first = vi.fn();
		const second = vi.fn();
		const other = vi.fn();
		await listenClient("review-comments-changed", first);
		await listenClient("review-comments-changed", second);
		await listenClient("git-status-changed", other);
		await vi.waitFor(() => expect(publish).toBeDefined());
		publish();
		await vi.waitFor(() => expect(second).toHaveBeenCalledOnce());
		expect(first).toHaveBeenCalledWith({ payload: "/repo" });
		expect(second.mock.calls[0][0].payload).toBe(
			first.mock.calls[0][0].payload,
		);
		expect(other).not.toHaveBeenCalled();
		expect(decode).toHaveBeenCalledOnce();
	} finally {
		decode.mockRestore();
	}
});

it("確立済みpushの途中resyncは両listenerへ復旧を通知し後続pushも配信する", async () => {
	const { listenClient } = await import("./client");
	let resync!: () => void;
	const fixture = connectFixture({
		async *subscribePush(_, context) {
			yield create(PushSchema, { event: { case: "resync", value: {} } });
			await new Promise<void>((resolve) => {
				resync = resolve;
			});
			yield create(PushSchema, { event: { case: "resync", value: {} } });
			yield create(PushSchema, {
				event: {
					case: "reviewCommentsChanged",
					value: { value: "/after-resync" },
				},
			});
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
		},
	});
	const refresh = vi.fn();
	const reconnect = vi.fn();
	const pushed = vi.fn();
	onClientRefresh(refresh);
	await listenClient("review-comments-changed", pushed, reconnect);
	await vi.waitFor(() => {
		expect(refresh).toHaveBeenCalledOnce();
		expect(reconnect).toHaveBeenCalledOnce();
		expect(resync).toBeDefined();
	});
	resync();
	await vi.waitFor(() =>
		expect(pushed).toHaveBeenCalledWith({ payload: "/after-resync" }),
	);
	expect(refresh).toHaveBeenCalledTimes(2);
	expect(reconnect).toHaveBeenCalledTimes(2);
	expect(
		fixture.requests.filter((request) =>
			request.url.endsWith("/SubscribePush"),
		),
	).toHaveLength(1);
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
		async getExternalEditor(_, context) {
			started();
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
			return { value: "" };
		},
	});
	expect(AbortSignal.any).toBeUndefined();
	const client = await getClient();
	const refreshed = vi.fn();
	onClientRefresh(refreshed);
	await vi.waitFor(() => expect(refreshed).toHaveBeenCalledOnce());
	const abort = new AbortController();
	const pending = client
		.getExternalEditor({}, { signal: abort.signal })
		.catch((error) => error);
	await vi.waitFor(() => expect(started).toHaveBeenCalledOnce());
	abort.abort();
	expect(await pending).toMatchObject({ code: Code.Canceled });
	const streaming = fixture.requests.find((request) =>
		request.url.endsWith("/SubscribePush"),
	);
	expect(streaming?.signal.aborted).toBe(false);
	refreshClient();
	expect(streaming?.signal.aborted).toBe(true);
});

it.each(["start_watching"] as const)(
	"%sの再登録待ちに変わった状態をonReadyの後に再取得する",
	async () => {
		const { watchClient, refreshClient, listenClient } = await import(
			"./client"
		);
		let value = false;
		let registration = 0;
		let complete!: () => void;
		let publish!: () => void;
		const watch = async () => {
			if (++registration === 2)
				await new Promise<void>((resolve) => {
					complete = resolve;
				});
			return { value: BigInt(registration) };
		};
		connectFixture({
			watchFiles: watch,
			stopWatching: () => ({}),
			getPerformanceTelemetryEnabled: () => ({ value }),
			async *subscribePush(_, context) {
				yield create(PushSchema, { event: { case: "resync", value: {} } });
				await new Promise<void>((resolve) => {
					publish = resolve;
					context.signal.addEventListener("abort", () => resolve(), {
						once: true,
					});
				});
				if (context.signal.aborted) return;
				yield create(PushSchema, {
					event: {
						case: "reviewCommentsChanged",
						value: { value: "/current" },
					},
				});
				await new Promise<void>((resolve) =>
					context.signal.addEventListener("abort", () => resolve(), {
						once: true,
					}),
				);
			},
		});
		const pushed = vi.fn();
		const unlisten = await listenClient("review-comments-changed", pushed);
		let watcherId = 0;
		const observed: Array<{ watcherId: number; value: boolean }> = [];
		const off = onClientRefresh(() => {
			const id = watcherId;
			void invokeClient("get_performance_telemetry_enabled").then((value) =>
				observed.push({ watcherId: id, value }),
			);
		});
		const stop = watchClient({ path: "/repo" }, (id) => {
			watcherId = id;
		});
		await vi.waitFor(() => expect(watcherId).toBe(1));
		observed.length = 0;
		refreshClient();
		await vi.waitFor(
			() => {
				expect(complete).toBeDefined();
				expect(observed).toEqual([]);
			},
			{ timeout: 3000 },
		);
		publish();
		await vi.waitFor(() =>
			expect(pushed).toHaveBeenCalledWith({ payload: "/current" }),
		);
		value = true;
		complete();
		await vi.waitFor(() =>
			expect(observed).toContainEqual({ watcherId: 2, value: true }),
		);
		stop();
		off();
		unlisten();
	},
);

it("watcher復旧後の再取得通知が失敗してもpushを再購読する", async () => {
	const fixture = connectFixture();
	const refreshed = vi.fn(() => {
		if (refreshed.mock.calls.length === 1) throw new Error("refresh failed");
	});
	const off = onClientRefresh(refreshed);
	await vi.waitFor(() => expect(refreshed).toHaveBeenCalledTimes(2), {
		timeout: 3000,
	});
	expect(
		fixture.requests.filter((request) =>
			request.url.endsWith("/SubscribePush"),
		),
	).toHaveLength(2);
	off();
});

it("新規watcher登録は購読全体へ再接続を通知しない", async () => {
	const { watchClient } = await import("./client");
	connectFixture({
		watchFiles: () => ({ value: 1n }),
		stopWatching: () => ({}),
	});
	const refreshed = vi.fn();
	const off = onClientRefresh(refreshed);
	await vi.waitFor(() => expect(refreshed).toHaveBeenCalledOnce());
	const ready = vi.fn();
	const stop = watchClient({ path: "/repo" }, ready);
	await vi.waitFor(() => expect(ready).toHaveBeenCalledWith(1));
	expect(refreshed).toHaveBeenCalledOnce();
	stop();
	off();
});

type StateEvent = MessageInitShape<typeof StateSubscriptionEventSchema>;

function stateFixture(start?: () => Promise<Record<string, never>>) {
	const streams: {
		send: (event: StateEvent) => void;
		fail: () => void;
		signal: AbortSignal;
	}[] = [];
	const starts: StartStateSubscriptionRequest[] = [];
	const reports = vi.fn(() => ({}));
	const stops = vi.fn((_request: { target: string }) => ({}));
	connectFixture({
		async *openStateStream(_, context) {
			const queue: (StateEvent | Error)[] = [];
			let wake = () => {};
			const push = (item: StateEvent | Error) => {
				queue.push(item);
				wake();
			};
			streams.push({
				send: push,
				fail: () => push(new ConnectError("state lost", Code.Unavailable)),
				signal: context.signal,
			});
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
	return { streams, starts, stops, reports };
}

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

it("状態のstreamが切れたら最後に受け取った版から購読を再開する", async () => {
	const fixture = stateFixture();
	const receive = vi.fn();
	subscribeState("repository-paths", receive);
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(1));
	fixture.streams[0].send(repositoryPaths(0, ["/a"], "snapshot"));
	fixture.streams[0].send(repositoryPaths(2, ["/b"]));
	await vi.waitFor(() => expect(receive).toHaveBeenCalledWith(["/b"]));
	fixture.streams[0].fail();
	await vi.waitFor(() => expect(fixture.starts).toHaveLength(2), {
		timeout: 3000,
	});
	expect(fixture.starts[1].version).toMatchObject({
		epoch: "boot",
		sequence: 2n,
	});
	fixture.streams[1].send(repositoryPaths(3, ["/c"]));
	await vi.waitFor(() => expect(receive).toHaveBeenLastCalledWith(["/c"]));
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
