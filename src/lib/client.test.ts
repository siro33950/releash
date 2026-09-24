import { create, type MessageInitShape } from "@bufbuild/protobuf";
import { Code, ConnectError, type HandlerContext } from "@connectrpc/connect";
import { afterEach, expect, it, vi } from "vitest";
import {
	CommandErrorSchema,
	PushSchema,
	type StartStateSubscriptionRequest,
	type StateSubscriptionEventSchema,
	type TerminalSubscriptionEventSchema,
} from "@/generated/client_pb";
import { connectFixture } from "@/test/connect";
import { invokeClient, onClientRefresh, subscribeState } from "./client";
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
	const fixture = connectFixture({ getCwd: read });
	await expect(invokeClient("get_cwd")).resolves.toEqual("/repo");
	expect(read).toHaveBeenCalledOnce();
	expect(
		fixture.requests.map((request) => new URL(request.url).pathname),
	).toEqual([
		"/releash.client.v1.ClientService/GetServerInfo",
		"/releash.client.v1.ClientService/GetCwd",
	]);
});

it("応答未到達の変更要求を終了し再接続後に現在状態だけを再取得する", async () => {
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
	await expect(
		invokeClient("update_crash_reporting", { enabled: true }),
	).rejects.toBeInstanceOf(ConnectError);
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
	const stop = watchClient("start_watching", { path: "/repo" }, ready, vi.fn());
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
	const stop = watchClient("start_watching", { path: "/repo" }, ready);
	await vi.waitFor(() => expect(registered).toHaveBeenCalledOnce());
	stop();
	complete();
	await vi.waitFor(() => expect(stopped).toHaveBeenCalledOnce());
	expect(ready).not.toHaveBeenCalled();
});

it.each(["start_watching", "start_git_dir_watching"] as const)(
	"%sは再購読時の上限超過後も枠が空けば同じ購読で監視を復旧する",
	async (command) => {
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
			watchGitDirectory: watch,
			stopWatching: () => ({}),
		});
		const ready = vi.fn();
		const failed = vi.fn();
		const stop = watchClient(
			command,
			command === "start_watching" ? { path: "/repo" } : { repoPath: "/repo" },
			ready,
			failed,
		);
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
		const stop = watchClient(
			"start_watching",
			{ path: "/repo" },
			vi.fn(),
			failed,
		);
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

it.each(["snapshot", "exit", "failure", "end"])(
	"terminal streamの%sを正常終了と切断に分ける",
	async (ending) => {
		const { attachClientStream } = await import("./client");
		connectFixture({
			async *terminalOutput() {
				yield {
					item: {
						case: "snapshot",
						value: {
							sessionKey: "terminal",
							isExited: ending === "snapshot",
							exitCode: ending === "snapshot" ? 3 : undefined,
						},
					},
				};
				if (ending === "failure")
					throw new ConnectError("lost", Code.Unavailable);
				if (ending === "exit")
					yield {
						item: {
							case: "exit",
							value: { sessionKey: "terminal", exitCode: 0, sequence: 1n },
						},
					};
			},
		});
		const received = vi.fn();
		const closed = vi.fn();
		const release = await attachClientStream(
			{
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "terminal",
				recovery: false,
			},
			received,
			closed,
		);
		if (ending === "failure" || ending === "end")
			await vi.waitFor(() => expect(closed).toHaveBeenCalledOnce());
		else {
			await new Promise((resolve) => setTimeout(resolve, 20));
			expect(closed).not.toHaveBeenCalled();
			expect(received).toHaveBeenCalledTimes(ending === "exit" ? 2 : 1);
		}
		await release();
	},
);

it("terminalの途中完了から同じ接続で再attachしsnapshotと後続出力を受信する", async () => {
	const { attachClientStream, getClient } = await import("./client");
	const requests: Array<{ attachmentId?: string; recovery?: boolean }> = [];
	connectFixture({
		async *terminalOutput(request, context) {
			requests.push(request);
			yield {
				item: {
					case: "snapshot",
					value: {
						sessionKey: "terminal",
						replay: request.recovery
							? "recovered snapshot"
							: "initial snapshot",
						sequence: request.recovery ? 42n : 0n,
						isExited: false,
					},
				},
			};
			if (!request.recovery) return;
			yield {
				item: {
					case: "output",
					value: { sessionKey: "terminal", data: "live output", sequence: 43n },
				},
			};
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
		},
	});
	const client = await getClient();
	const owner = { kind: "workspace", workspacePath: "/repo" } as const;
	const received = vi.fn();
	let recovery: Promise<() => Promise<void>> | undefined;
	const closed = vi.fn(() => {
		recovery = attachClientStream(
			{ owner, attachmentId: "recovered", recovery: true },
			received,
			closed,
		);
	});
	const release = await attachClientStream(
		{ owner, attachmentId: "initial", recovery: false },
		received,
		closed,
	);
	await vi.waitFor(() => expect(received).toHaveBeenCalledTimes(3));
	expect(closed).toHaveBeenCalledOnce();
	expect(requests).toMatchObject([
		{ attachmentId: "initial", recovery: false },
		{ attachmentId: "recovered", recovery: true },
	]);
	expect(received.mock.calls[1][0]).toMatchObject({
		type: "snapshot",
		surface: {
			terminal_surface: { replay: "recovered snapshot", sequence: 42 },
		},
	});
	expect(received.mock.calls[2][0]).toEqual({
		type: "output",
		session_key: "terminal",
		data: "live output",
		sequence: 43,
	});
	expect(await getClient()).toBe(client);
	await release();
	await (await recovery)?.();
});

it.each(["end", "failure"])(
	"明示解除後のterminal streamの%sでは再同期を通知しない",
	async (ending) => {
		const { attachClientStream } = await import("./client");
		connectFixture({
			async *terminalOutput(_, context) {
				yield {
					item: {
						case: "snapshot",
						value: { sessionKey: "terminal", isExited: false },
					},
				};
				await new Promise<void>((resolve) =>
					context.signal.addEventListener("abort", () => resolve(), {
						once: true,
					}),
				);
				if (ending === "failure")
					throw new ConnectError("stream canceled", Code.Canceled);
			},
		});
		const closed = vi.fn();
		const release = await attachClientStream(
			{
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "terminal",
				recovery: false,
			},
			vi.fn(),
			closed,
		);
		await release();
		await new Promise((resolve) => setTimeout(resolve, 20));
		expect(closed).not.toHaveBeenCalled();
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
	const fixture = connectFixture({ getCwd: read });
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
	const pending = invokeClient("get_cwd");
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
		getCwd: () => {
			throw commandError("REPOSITORY_UNAVAILABLE", "Repository is unavailable");
		},
	});
	const client = await getClient();
	const error = await invokeClient("get_cwd").catch((error) => error);
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
])(
	"接続断でないcode %sは共有接続と進行中のRPCとpushを中断しない",
	async (code) => {
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
			getCwd: read,
			updateExternalEditor: reject,
			watchFiles: reject,
			watchGitDirectory: reject,
		});
		const refreshed = vi.fn();
		onClientRefresh(refreshed);
		await vi.waitFor(() => expect(refreshed).toHaveBeenCalledOnce());
		const client = await getClient();
		const pending = invokeClient("get_cwd");
		await vi.waitFor(() => expect(read).toHaveBeenCalledOnce());
		const error = await invokeClient("update_external_editor", {
			editor: "vim",
		}).catch((error) => error);
		expect(error).toBeInstanceOf(ConnectError);
		expect(error).toMatchObject({ code, rawMessage: "request rejected" });
		for (const command of [
			"start_watching",
			"start_git_dir_watching",
		] as const) {
			const failed = vi.fn();
			const stop = watchClient(
				command,
				command === "start_watching"
					? { path: "/repo" }
					: { repoPath: "/repo" },
				vi.fn(),
				failed,
			);
			await vi.waitFor(() => expect(failed).toHaveBeenCalledOnce());
			expect(failed.mock.calls[0][0]).toMatchObject({ code });
			stop();
		}
		expect(await getClient()).toBe(client);
		expect(
			fixture.requests.find((request) => request.url.endsWith("/GetCwd"))
				?.signal.aborted,
		).toBe(false);
		expect(
			fixture.requests.find((request) => request.url.endsWith("/SubscribePush"))
				?.signal.aborted,
		).toBe(false);
		expect(refreshed).toHaveBeenCalledOnce();
		await release();
		await expect(pending).resolves.toEqual("/repo");
	},
);

it.each(["detail", "transport", "end"])(
	"terminal初回受信の%sは失敗値の型と理由を保持する",
	async (failure) => {
		const { attachClientStream } = await import("./client");
		connectFixture({
			async *terminalOutput() {
				if (failure === "detail")
					throw commandError(
						"TERMINAL_ATTACHMENT_LIMIT",
						"Too many terminal attachments",
						Code.ResourceExhausted,
					);
				if (failure === "transport")
					throw new ConnectError("connection lost", Code.Unavailable);
				yield* [];
			},
		});
		const received = vi.fn();
		const closed = vi.fn();
		const error = await attachClientStream(
			{
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "terminal",
				recovery: false,
			},
			received,
			closed,
		).catch((error) => error);
		if (failure === "detail") {
			expect(error).not.toBeInstanceOf(Error);
			expect(error).toStrictEqual({
				code: "TERMINAL_ATTACHMENT_LIMIT",
				message: "Too many terminal attachments",
			});
			expect(getErrorMessage(error)).toBe("Too many terminal attachments");
		} else if (failure === "transport") {
			expect(error).toBeInstanceOf(ConnectError);
			expect(error).toMatchObject({
				code: Code.Unavailable,
				message: "[unavailable] connection lost",
				rawMessage: "connection lost",
			});
			expect(getErrorMessage(error)).toBe("処理中にエラーが発生しました");
		} else {
			expect(error).toBeInstanceOf(Error);
			expect(error).not.toBeInstanceOf(ConnectError);
			expect(error.message).toBe("Terminal stream closed before snapshot");
			expect(error).not.toHaveProperty("code");
		}
		expect(received).not.toHaveBeenCalled();
		expect(closed).toHaveBeenCalledTimes(failure === "end" ? 1 : 0);
	},
);

it.each(["unavailable", "network"])(
	"terminal初回受信の%sから接続通知で再アタッチしsnapshotを取得する",
	async (failure) => {
		vi.useFakeTimers();
		const { attachClientStream, onClientConnection } = await import("./client");
		let attempts = 0;
		const fixture = connectFixture({
			async *terminalOutput(_, context) {
				if (++attempts <= 2)
					throw new ConnectError("terminal response lost", Code.Unavailable);
				yield {
					item: {
						case: "snapshot",
						value: {
							sessionKey: "terminal",
							replay: "current output",
							sequence: 42n,
							cols: 80,
							rows: 24,
							isExited: false,
						},
					},
				};
				await new Promise<void>((resolve) =>
					context.signal.addEventListener("abort", () => resolve(), {
						once: true,
					}),
				);
			},
		});
		if (failure === "network") {
			const fetch = fixture.fetch.getMockImplementation();
			if (!fetch) throw new Error("Missing fixture implementation");
			fixture.fetch.mockImplementation((input, init) => {
				const request = new Request(input, init);
				if (request.url.endsWith("/AttachTerminalSurface") && attempts < 2) {
					attempts++;
					throw new TypeError("terminal response lost");
				}
				return fetch(request);
			});
		}
		const received = vi.fn();
		const failed = vi.fn();
		const closed = vi.fn();
		const refreshed = vi.fn();
		onClientRefresh(refreshed);
		const release = onClientConnection((connected) => {
			if (!connected || attempts === 0) return;
			void attachClientStream(
				{
					owner: { kind: "workspace", workspacePath: "/repo" },
					attachmentId: crypto.randomUUID(),
					recovery: true,
				},
				received,
				closed,
			).catch(failed);
		});
		try {
			await vi.waitFor(() => expect(refreshed).toHaveBeenCalledOnce());
			await attachClientStream(
				{
					owner: { kind: "workspace", workspacePath: "/repo" },
					attachmentId: "initial",
					recovery: false,
				},
				received,
				closed,
			).catch(failed);
			await vi.waitFor(() => expect(received).toHaveBeenCalledOnce(), {
				timeout: 4000,
			});
			expect(failed).toHaveBeenCalledTimes(2);
			expect(attempts).toBe(3);
			expect(received).toHaveBeenCalledWith({
				type: "snapshot",
				surface: {
					session_key: "terminal",
					terminal_surface: {
						replay: "current output",
						sequence: 42,
						cols: 80,
						rows: 24,
					},
					is_exited: false,
					exit_code: null,
				},
			});
			expect(closed).not.toHaveBeenCalled();
		} finally {
			release();
			window.dispatchEvent(new Event("pagehide"));
			await vi.advanceTimersByTimeAsync(1000);
			vi.useRealTimers();
		}
	},
);

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
		const stop = watchClient("start_watching", { path: "/repo" }, vi.fn());
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
		await listenClient("branch-list-sync", other);
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

it("terminal共有購読の終了後に全attachmentを再同期しpushとunaryを維持する", async () => {
	const { attachClientStream, getClient } = await import("./client");
	const fixture = connectFixture({
		getCwd: () => ({ value: "/current" }),
		async *terminalOutput(request, context) {
			yield {
				item: {
					case: "snapshot",
					value: {
						sessionKey: request.attachmentId,
						replay: request.recovery ? "current" : "initial",
					},
				},
			};
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
		},
	});
	const client = await getClient();
	const received = [vi.fn(), vi.fn()];
	const releases: Array<() => Promise<void>> = [];
	const recovered: Array<Promise<() => Promise<void>>> = [];
	for (let index = 0; index < 2; index++) {
		const owner = {
			kind: "workspace",
			workspacePath: `/repo-${index}`,
		} as const;
		releases.push(
			await attachClientStream(
				{ owner, attachmentId: `initial-${index}`, recovery: false },
				received[index],
				() => {
					recovered.push(
						attachClientStream(
							{ owner, attachmentId: `recovered-${index}`, recovery: true },
							received[index],
							vi.fn(),
						),
					);
				},
			),
		);
	}
	fixture.closeTerminalSubscriptions();
	await vi.waitFor(() => expect(recovered).toHaveLength(2));
	releases.push(...(await Promise.all(recovered)));
	for (const receive of received) {
		expect(receive).toHaveBeenCalledTimes(2);
		expect(receive.mock.calls[1][0]).toMatchObject({
			type: "snapshot",
			surface: { terminal_surface: { replay: "current" } },
		});
	}
	expect(await getClient()).toBe(client);
	await expect(invokeClient("get_cwd")).resolves.toEqual("/current");
	expect(
		fixture.requests.filter((request) =>
			request.url.endsWith("/SubscribeTerminalSurfaces"),
		),
	).toHaveLength(2);
	for (const release of releases) await release();
});

it.each(["rejected", "end"])(
	"terminal共有購読の%sは待機中attachmentを失敗させる",
	async (failure) => {
		const { attachClientStream } = await import("./client");
		connectFixture({
			async *subscribeTerminalSurfaces() {
				if (failure === "rejected")
					throw new ConnectError("subscription limit", Code.ResourceExhausted);
				yield* [];
			},
		});
		const received = vi.fn();
		const closed = vi.fn();
		const error = await attachClientStream(
			{
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "pending",
				recovery: false,
			},
			received,
			closed,
		).catch((error) => error);
		if (failure === "rejected")
			expect(error).toMatchObject({ code: Code.ResourceExhausted });
		else
			expect(error.message).toBe("Terminal subscription closed before ready");
		expect(received).not.toHaveBeenCalled();
		expect(closed).not.toHaveBeenCalled();
	},
);

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
		async getCwd(_, context) {
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
		.getCwd({}, { signal: abort.signal })
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

it("同じattachmentへの旧Closedと旧releaseは新しいlistenerと入力を壊さない", async () => {
	const { attachClientStream, getClient } = await import("./client");
	let endOld!: () => void;
	let outputNew!: () => void;
	let attempts = 0;
	const write = vi.fn(() => ({}));
	const fixture = connectFixture({
		writeTerminalSurface: write,
		async *terminalOutput(_, context) {
			const first = ++attempts === 1;
			yield { item: { case: "snapshot", value: { sessionKey: "same" } } };
			if (first) {
				await new Promise<void>((resolve) => {
					endOld = resolve;
				});
				return;
			}
			await new Promise<void>((resolve) => {
				outputNew = resolve;
			});
			yield {
				item: {
					case: "output",
					value: { sessionKey: "same", sequence: 1n, data: "new output" },
				},
			};
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
		},
	});
	const args = {
		owner: { kind: "workspace", workspacePath: "/repo" },
		attachmentId: "same",
		recovery: false,
	} as const;
	const oldClosed = vi.fn();
	const newClosed = vi.fn();
	const received = vi.fn();
	const oldRelease = await attachClientStream(args, vi.fn(), oldClosed);
	const newRelease = await attachClientStream(args, received, newClosed);
	endOld();
	await oldRelease();
	outputNew();
	await vi.waitFor(() => expect(received).toHaveBeenCalledTimes(2));
	expect(newClosed).not.toHaveBeenCalled();
	expect(oldClosed).not.toHaveBeenCalled();
	await (await getClient()).writeTerminalSurface({
		attachmentId: "same",
		sequence: 0n,
		data: "input",
	});
	expect(write).toHaveBeenCalledOnce();
	expect(
		fixture.requests.filter((request) =>
			request.url.endsWith("/DetachTerminalSurface"),
		),
	).toHaveLength(0);
	await Promise.all([newRelease(), newRelease()]);
	expect(
		fixture.requests.filter((request) =>
			request.url.endsWith("/DetachTerminalSurface"),
		),
	).toHaveLength(1);
});

it.each(["disconnect", "pending"])(
	"接続断時の%sなattachment解放はエラーや旧接続への再送を発生させない",
	async (timing) => {
		const { attachClientStream, onClientConnection, refreshClient } =
			await import("./client");
		const detach = vi.fn(async (_, context: HandlerContext) => {
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
			return {};
		});
		const fixture = connectFixture({
			detachTerminalSurface: detach,
			async *terminalOutput(_, context) {
				yield { item: { case: "snapshot", value: { sessionKey: "terminal" } } };
				await new Promise<void>((resolve) =>
					context.signal.addEventListener("abort", () => resolve(), {
						once: true,
					}),
				);
			},
		});
		const closed = vi.fn();
		const release = await attachClientStream(
			{
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "terminal",
				recovery: false,
			},
			vi.fn(),
			closed,
		);
		let released: Promise<void> | undefined;
		const stop = onClientConnection((connected) => {
			if (!connected) released = release();
		});
		if (timing === "pending") {
			released = release();
			await vi.waitFor(() => expect(detach).toHaveBeenCalledOnce());
		}
		refreshClient();
		expect(released).toBeDefined();
		await expect(released).resolves.toBeUndefined();
		await expect(release()).resolves.toBeUndefined();
		expect(closed).not.toHaveBeenCalled();
		expect(
			fixture.requests.filter((request) =>
				request.url.endsWith("/DetachTerminalSurface"),
			),
		).toHaveLength(timing === "pending" ? 1 : 0);
		stop();
	},
);

it("解放RPCの失敗を呼び出し元へ返し繰り返し解放でも再送しない", async () => {
	const { attachClientStream } = await import("./client");
	const detach = vi.fn(() => {
		throw commandError("PTY_ERROR", "Terminal detachment failed. Try again.");
	});
	connectFixture({
		detachTerminalSurface: detach,
		async *terminalOutput(_, context) {
			yield { item: { case: "snapshot", value: { sessionKey: "terminal" } } };
			await new Promise<void>((resolve) =>
				context.signal.addEventListener("abort", () => resolve(), {
					once: true,
				}),
			);
		},
	});
	const release = await attachClientStream(
		{
			owner: { kind: "workspace", workspacePath: "/repo" },
			attachmentId: "terminal",
			recovery: false,
		},
		vi.fn(),
		vi.fn(),
	);
	for (let attempt = 0; attempt < 2; attempt++)
		await expect(release()).rejects.toEqual({
			code: "PTY_ERROR",
			message: "Terminal detachment failed. Try again.",
		});
	expect(detach).toHaveBeenCalledOnce();
});

it.each([false, true])(
	"terminal完了の再同期可否%sはsnapshotの状態ではなくRustの通知に従う",
	async (resynchronize) => {
		const { attachClientStream } = await import("./client");
		let attachment!: { attachmentId: string; streamId: string };
		let attached!: () => void;
		let finish!: () => void;
		const ready = new Promise<void>((resolve) => {
			attached = resolve;
		});
		const closed = new Promise<void>((resolve) => {
			finish = resolve;
		});
		connectFixture({
			attachTerminalSurface(request) {
				attachment = {
					attachmentId: request.request?.attachmentId ?? "",
					streamId: request.streamId,
				};
				attached();
				return {};
			},
			async *subscribeTerminalSurfaces(
				_,
				context,
			): AsyncIterable<
				MessageInitShape<typeof TerminalSubscriptionEventSchema>
			> {
				yield { event: { case: "ready", value: {} } };
				await ready;
				yield {
					...attachment,
					event: {
						case: "item",
						value: {
							item: {
								case: "snapshot",
								value: { sessionKey: "terminal", isExited: resynchronize },
							},
						},
					},
				};
				await closed;
				yield {
					...attachment,
					event: { case: "closed", value: { resynchronize } },
				};
				await new Promise<void>((resolve) =>
					context.signal.addEventListener("abort", () => resolve(), {
						once: true,
					}),
				);
			},
		});
		const onClosed = vi.fn();
		const release = await attachClientStream(
			{
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "terminal",
				recovery: false,
			},
			vi.fn(),
			onClosed,
		);
		finish();
		if (resynchronize)
			await vi.waitFor(() => expect(onClosed).toHaveBeenCalledOnce());
		else {
			await new Promise((resolve) => setTimeout(resolve, 20));
			expect(onClosed).not.toHaveBeenCalled();
		}
		await release();
	},
);

it.each(["start_watching", "start_git_dir_watching"] as const)(
	"%sの再登録待ちに変わった状態をonReadyの後に再取得する",
	async (command) => {
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
			watchGitDirectory: watch,
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
		const stop = watchClient(
			command,
			command === "start_watching" ? { path: "/repo" } : { repoPath: "/repo" },
			(id) => {
				watcherId = id;
			},
		);
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

it.each(["end", "closed"])(
	"初回snapshot前の%sから再attachしてsnapshotと出力と入力を復旧する",
	async (ending) => {
		const { attachClientStream, getClient } = await import("./client");
		let finish!: () => void;
		const pending = new Promise<void>((resolve) => {
			finish = resolve;
		});
		const write = vi.fn(() => ({}));
		const fixture = connectFixture({
			async *terminalOutput(request, context) {
				if (!request.recovery) {
					await pending;
					if (ending === "end") {
						fixture.closeTerminalSubscriptions();
						await new Promise<void>((resolve) =>
							context.signal.addEventListener("abort", () => resolve(), {
								once: true,
							}),
						);
					}
					return;
				}
				yield {
					item: {
						case: "snapshot",
						value: { sessionKey: "terminal", replay: "restored", sequence: 3n },
					},
				};
				yield {
					item: {
						case: "output",
						value: { sessionKey: "terminal", data: "live", sequence: 4n },
					},
				};
				await new Promise<void>((resolve) =>
					context.signal.addEventListener("abort", () => resolve(), {
						once: true,
					}),
				);
			},
			writeTerminalSurface: write,
		});
		const received = vi.fn();
		let recovery: Promise<() => Promise<void>> | undefined;
		const owner = { kind: "workspace", workspacePath: "/repo" } as const;
		const onClosed = vi.fn(() => {
			recovery = attachClientStream(
				{ owner, attachmentId: "recovered", recovery: true },
				received,
				vi.fn(),
			);
		});
		const initial = attachClientStream(
			{ owner, attachmentId: "initial", recovery: false },
			received,
			onClosed,
		).catch((error) => error);
		await vi.waitFor(() =>
			expect(
				fixture.requests.some((request) =>
					request.url.endsWith("/AttachTerminalSurface"),
				),
			).toBe(true),
		);
		finish();
		await vi.waitFor(() => expect(onClosed).toHaveBeenCalledOnce());
		expect(await initial).toBeInstanceOf(Error);
		const release = await recovery;
		await vi.waitFor(() =>
			expect(received).toHaveBeenCalledWith(
				expect.objectContaining({ type: "output", data: "live" }),
			),
		);
		expect(received).toHaveBeenCalledWith(
			expect.objectContaining({
				type: "snapshot",
				surface: expect.objectContaining({
					terminal_surface: expect.objectContaining({ replay: "restored" }),
				}),
			}),
		);
		await invokeClient("write_terminal_surface", {
			owner,
			attachmentId: "recovered",
			sequence: 0,
			data: "input",
			clientStartedAtUnixMs: null,
		});
		expect(write).toHaveBeenCalledWith(
			expect.objectContaining({ attachmentId: "recovered", data: "input" }),
			expect.anything(),
		);
		expect(await getClient()).toBeDefined();
		await release?.();
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
	const stop = watchClient("start_watching", { path: "/repo" }, ready);
	await vi.waitFor(() => expect(ready).toHaveBeenCalledWith(1));
	expect(refreshed).toHaveBeenCalledOnce();
	stop();
	off();
});

type StateEvent = MessageInitShape<typeof StateSubscriptionEventSchema>;

function stateFixture() {
	const streams: {
		send: (event: StateEvent) => void;
		fail: () => void;
		signal: AbortSignal;
	}[] = [];
	const starts: StartStateSubscriptionRequest[] = [];
	const stops = vi.fn(() => ({}));
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
			return {};
		},
		stopStateSubscription: stops,
	});
	return { streams, starts, stops };
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
