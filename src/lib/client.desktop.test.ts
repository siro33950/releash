import type { MessageInitShape } from "@bufbuild/protobuf";
import { Code, ConnectError, type HandlerContext } from "@connectrpc/connect";
import { invoke } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { StateSubscriptionEventSchema } from "@/generated/client_pb";
import { useBackgroundConfig } from "@/hooks/useAppSettings";
import { connectFixture } from "@/test/connect";
import {
	completeClientRestoration,
	getClient,
	invokeClient,
	subscribeTerminalState,
} from "./client";

vi.unmock("@/lib/client");
afterEach(async () => {
	window.dispatchEvent(new Event("pagehide"));
	await new Promise((resolve) => setTimeout(resolve, 0));
	vi.unstubAllGlobals();
});

it("desktopの業務要求をHTTPへ送りIPCには接続情報と復元完了だけを渡す", async () => {
	vi.mocked(invoke).mockClear();
	const fixture = connectFixture({
		getExternalEditor: () => ({ value: "/repo" }),
	});
	await expect(invokeClient("get_external_editor")).resolves.toEqual("/repo");
	await completeClientRestoration(7);
	for (const request of fixture.requests)
		expect(request.headers.get("authorization")).toBe("Bearer client-token");
	expect(vi.mocked(invoke).mock.calls.map(([name]) => name)).toEqual([
		"get_client_endpoint",
		"validate_daemon_connection",
		"complete_desktop_restoration",
	]);
	expect(invoke).toHaveBeenLastCalledWith("complete_desktop_restoration", {
		launchId: "launch",
		attachmentId: expect.any(String),
		generation: 7,
	});
});

it("破棄済み画面の遅い接続情報が次の接続を上書きしない", async () => {
	connectFixture({ getExternalEditor: () => ({ value: "/repo" }) });
	const original = vi.mocked(invoke).getMockImplementation();
	if (!original) throw new Error("Missing endpoint fixture");
	let release!: (value: unknown) => void;
	let endpoints = 0;
	vi.mocked(invoke).mockImplementation((command, args) => {
		if (command === "get_client_endpoint" && ++endpoints === 1)
			return new Promise((resolve) => {
				release = resolve;
			});
		return original(command, args);
	});
	const old = invokeClient("get_external_editor").catch((error) => error);
	window.dispatchEvent(new Event("pagehide"));
	await expect(invokeClient("get_external_editor")).resolves.toEqual("/repo");
	release({ url: "http://127.0.0.1:9829", token: "old", launchId: "launch" });
	expect(await old).toBeInstanceOf(Error);
	await expect(invokeClient("get_external_editor")).resolves.toEqual("/repo");
	expect(endpoints).toBe(2);
});

it("Rustの同一性検証が失敗した接続では業務RPCも復元完了も呼ばない", async () => {
	const read = vi.fn(() => ({ value: "/repo" }));
	connectFixture({
		getServerInfo: () => ({ launchId: "different", release: "test" }),
		getExternalEditor: read,
	});
	const original = vi.mocked(invoke).getMockImplementation();
	if (!original) throw new Error("Missing fixture implementation");
	vi.mocked(invoke).mockClear();
	vi.mocked(invoke).mockImplementation(async (command, args) => {
		if (command === "validate_daemon_connection")
			throw new Error("Daemon identity changed");
		return original(command, args);
	});
	await expect(invokeClient("get_external_editor")).rejects.toThrow(
		"Daemon identity changed",
	);
	await expect(completeClientRestoration(7)).rejects.toThrow(
		"Daemon identity changed",
	);
	expect(invoke).toHaveBeenCalledWith("validate_daemon_connection", {
		launchId: "different",
		release: "test",
	});
	expect(read).not.toHaveBeenCalled();
	expect(
		vi
			.mocked(invoke)
			.mock.calls.some(([name]) => name === "complete_desktop_restoration"),
	).toBe(false);
});

it("設定再適用はコマンド名によらずRustの応答指示に従う", async () => {
	const settings = {
		closeToTray: false,
		startMinimized: true,
		crashReporting: true,
		performanceTelemetry: false,
	};
	const fixture = connectFixture({
		getServerInfo: () => ({
			launchId: "launch",
			release: "test",
			desktopSettings: settings,
		}),
		updateCrashReporting: () => ({}),
		getExternalEditor: (_, context) => {
			context.responseHeader.set("releash-desktop-settings-changed", "true");
			return { value: "/repo" };
		},
	});
	const { getClient } = await import("./client");
	await getClient();
	vi.mocked(invoke).mockClear();
	await invokeClient("update_crash_reporting", { enabled: true });
	expect(invoke).not.toHaveBeenCalled();
	await expect(invokeClient("get_external_editor")).resolves.toEqual("/repo");
	expect(invoke).toHaveBeenCalledExactlyOnceWith("apply_desktop_settings", {
		settings: expect.objectContaining(settings),
	});
	expect(
		fixture.requests.filter((request) =>
			request.url.endsWith("/GetServerInfo"),
		),
	).toHaveLength(2);
});

it.each(["unavailable", "network", "permission", "apply"])(
	"保存後の設定同期の%s失敗は保存結果と進行中RPCとterminal出力を損なわない",
	async (failure) => {
		let saved = false;
		let finishRead!: () => void;
		let emitOutput!: () => void;
		let startTerminal!: () => void;
		const started = new Promise<void>((resolve) => {
			startTerminal = resolve;
		});
		const settings = {
			closeToTray: true,
			startMinimized: false,
			crashReporting: false,
			performanceTelemetry: false,
		};
		const update = vi.fn((_: unknown, context: HandlerContext) => {
			saved = true;
			context.responseHeader.set("releash-desktop-settings-changed", "true");
			return {};
		});
		const fixture = connectFixture({
			getServerInfo: () => {
				if (saved && failure !== "apply")
					throw new ConnectError(
						"settings refresh failed",
						failure === "permission" ? Code.PermissionDenied : Code.Unavailable,
					);
				return {
					launchId: "launch",
					release: "test",
					desktopSettings: settings,
				};
			},
			getAppSettings: () => ({ closeToTray: true, startMinimized: false }),
			updateAppSettings: update,
			getExternalEditor: async () => {
				await new Promise<void>((resolve) => {
					finishRead = resolve;
				});
				return { value: "/repo" };
			},
			startStateSubscription: () => {
				startTerminal();
				return {};
			},
			stopStateSubscription: () => ({}),
			async *openStateStream(
				_,
				context,
			): AsyncGenerator<MessageInitShape<typeof StateSubscriptionEventSchema>> {
				yield { event: { case: "ready", value: {} } };
				await started;
				yield {
					target: "terminal",
					args: ["/repo"],
					version: { epoch: "terminal", sequence: 0n },
					event: {
						case: "snapshot",
						value: {
							value: {
								case: "terminal",
								value: {
									item: {
										case: "snapshot",
										value: {
											sessionKey: "terminal",
											isExited: false,
											processedReportUnits: 5000,
										},
									},
								},
							},
						},
					},
				};
				await new Promise<void>((resolve) => {
					emitOutput = resolve;
				});
				yield {
					target: "terminal",
					args: ["/repo"],
					version: { epoch: "terminal", sequence: 1n },
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
											value: {
												sessionKey: "terminal",
												data: "still running",
												sequence: 1n,
											},
										},
									},
								},
							},
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
		const originalInvoke = vi.mocked(invoke).getMockImplementation();
		const originalFetch = fixture.fetch.getMockImplementation();
		if (!originalInvoke || !originalFetch)
			throw new Error("Missing fixture implementation");
		vi.mocked(invoke).mockImplementation(async (command, args) => {
			if (command === "get_login_item_status")
				return { enabled: false, requiresApproval: false, reason: null };
			if (command === "apply_desktop_settings" && saved && failure === "apply")
				throw new Error("desktop apply failed");
			return originalInvoke(command, args);
		});
		if (failure === "network")
			fixture.fetch.mockImplementation((input, init) => {
				const request = new Request(input, init);
				if (saved && request.url.endsWith("/GetServerInfo"))
					throw new TypeError("settings refresh failed");
				return originalFetch(request);
			});
		const { result, unmount } = renderHook(() => useBackgroundConfig());
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.error).toBeNull();
		const client = await getClient();
		const pending = invokeClient("get_external_editor").catch((error) => error);
		const received = vi.fn();
		const closed = vi.fn();
		const release = await subscribeTerminalState(
			{
				owner: { kind: "workspace", workspacePath: "/repo" },
				attachmentId: "terminal",
			},
			received,
			closed,
		);
		try {
			await vi.waitFor(() => {
				expect(finishRead).toBeDefined();
				expect(emitOutput).toBeDefined();
			});
			act(() =>
				result.current.setDraft({
					...result.current.draft,
					close_to_tray: false,
				}),
			);
			await act(async () => {
				await expect(result.current.save()).resolves.toBeUndefined();
			});
			expect(result.current.error).toBeNull();
			expect(result.current.isDirty).toBe(false);
			expect(result.current.draft.close_to_tray).toBe(false);
			expect(result.current.saving).toBe(false);
			expect(update).toHaveBeenCalledOnce();
			expect(await getClient()).toBe(client);
			for (const method of [
				"GetExternalEditor",
				"SubscribePush",
				"OpenStateStream",
			])
				expect(
					fixture.requests.find((request) => request.url.endsWith(`/${method}`))
						?.signal.aborted,
				).toBe(false);
			finishRead();
			await expect(pending).resolves.toEqual("/repo");
			emitOutput();
			await vi.waitFor(() =>
				expect(received).toHaveBeenCalledWith({
					type: "output",
					session_key: "terminal",
					data: "still running",
					sequence: 1,
				}),
			);
			expect(closed).not.toHaveBeenCalled();
		} finally {
			finishRead?.();
			emitOutput?.();
			await release();
			unmount();
			await pending.catch(() => {});
		}
	},
);
