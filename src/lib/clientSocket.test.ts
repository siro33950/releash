import {
	create,
	fromBinary,
	fromJson,
	toBinary,
	toJson,
} from "@bufbuild/protobuf";
import {
	CommandErrorSchema,
	CommandRequestSchema,
	CommandResultSchema,
	EnvelopeSchema,
	PushSchema,
	TerminalEventSchema,
	WorkflowExecutionViewSchema,
	WorkflowValueSchema,
} from "@/generated/client_pb";

vi.unmock("./clientSocket");

import type { JsonValue } from "@bufbuild/protobuf";
import { invoke } from "@tauri-apps/api/core";
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useWorkflowState } from "@/hooks/useWorkflowState";
import { useWorkspaceStateCache } from "@/hooks/useWorkspaceStateCache";
import type { WorkflowExecution } from "@/types/workflow";
import type { WorkspaceState } from "@/types/workspace-state";
import { clientJson } from "./clientJson";
import {
	acknowledgeClientStream,
	invokeClient,
	listenClient,
	listenClientStream,
	onClientConnection,
} from "./clientSocket";

function execution(): WorkflowExecution {
	return {
		id: "execution-1",
		workflowName: "test",
		status: "running",
		currentNode: null,
		worktreePath: "/repo",
		createdFrom: "desktop_ui",
		startedAt: 1,
		updatedAt: 1,
		completedAt: null,
		errorReason: null,
		interruptionReason: null,
		resumeFromNode: null,
		totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
		nodeExecutions: [],
		artifacts: [],
		fanouts: [],
		approvalTarget: null,
	};
}

class FakeWebSocket {
	static instances: FakeWebSocket[] = [];
	static failOpen = false;
	static autoOpen = true;
	static respond: ((command: string) => unknown) | null = null;
	onopen: (() => void) | null = null;
	onerror: (() => void) | null = null;
	onclose: (() => void) | null = null;
	onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
	send = vi.fn((bytes: Uint8Array) => {
		if (!FakeWebSocket.respond) return;
		const body = fromBinary(EnvelopeSchema, bytes).body;
		if (body.case !== "request") return;
		const command = body.value.command.case;
		if (!command) throw new Error("Missing command");
		this.message({
			request_id: body.value.requestId,
			result: FakeWebSocket.respond(command),
		});
	});
	close = vi.fn();
	constructor(
		readonly url: string,
		readonly protocols: string[],
	) {
		FakeWebSocket.instances.push(this);
		queueMicrotask(() => {
			if (!FakeWebSocket.autoOpen) return;
			if (FakeWebSocket.failOpen) this.onerror?.();
			else this.onopen?.();
		});
	}
	message(frame: unknown) {
		const value = frame as Record<string, unknown>;
		let envelope: JsonValue;
		if (value.status === "push") {
			const field = PushSchema.fields.find(
				(field) => field.name.replace(/_/g, "-") === value.event,
			);
			if (!field?.message) throw new Error("Unknown push");
			envelope = {
				push: {
					[field.jsonName]: clientJson(
						field.message,
						value.payload as JsonValue,
						true,
					),
				},
			};
		} else {
			const request = this.send.mock.calls
				.map(([bytes]) => fromBinary(EnvelopeSchema, bytes).body)
				.find(
					(body) =>
						body.case === "request" &&
						body.value.requestId === value.request_id,
				);
			if (request?.case !== "request") throw new Error("Missing request");
			const field = CommandResultSchema.fields.find(
				(field) => field.localName === request.value.command.case,
			);
			if (!field?.message) throw new Error("Unknown command");
			envelope = {
				response: {
					requestId: String(value.request_id),
					...(value.error !== undefined
						? {
								error: clientJson(
									CommandErrorSchema,
									value.error as JsonValue,
									true,
								),
							}
						: {
								result: {
									[field.jsonName]: clientJson(
										field.message,
										value.result as JsonValue,
										true,
									),
								},
							}),
				},
			};
		}
		const bytes = toBinary(EnvelopeSchema, fromJson(EnvelopeSchema, envelope));
		this.onmessage?.({ data: bytes.buffer as ArrayBuffer });
	}
}

async function sent(count = 1, socketIndex = 0) {
	await vi.waitFor(() =>
		expect(FakeWebSocket.instances[socketIndex]?.send).toHaveBeenCalledTimes(
			count,
		),
	);
	const socket = FakeWebSocket.instances[socketIndex];
	return {
		socket,
		frames: socket.send.mock.calls.map(([frame]) => {
			const body = fromBinary(EnvelopeSchema, frame).body;
			if (body.case !== "request") throw new Error("Expected command");
			const command = body.value.command;
			const field = CommandRequestSchema.fields.find(
				(field) => field.localName === command.case,
			);
			if (!field?.message || !command.value) throw new Error("Missing command");
			return {
				request_id: body.value.requestId,
				command: field.name,
				args: clientJson(
					field.message,
					toJson(field.message, command.value),
					false,
				),
			};
		}),
	};
}

describe("clientSocket", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		FakeWebSocket.instances = [];
		FakeWebSocket.respond = null;
		FakeWebSocket.failOpen = false;
		FakeWebSocket.autoOpen = true;
		vi.stubGlobal("WebSocket", FakeWebSocket);
		vi.mocked(invoke).mockResolvedValue({
			url: "ws://127.0.0.1:123/v1/client",
			authSubprotocol: "releash-bearer.client",
		});
	});
	afterEach(() => {
		for (const socket of FakeWebSocket.instances) socket.onclose?.();
		vi.unstubAllGlobals();
		vi.useRealTimers();
		vi.restoreAllMocks();
	});
	it("1接続で複数要求をrequest_idで相関し逆順の応答を返す", async () => {
		const first = invokeClient("get_current_branch", { repoPath: "/a" });
		const second = invokeClient("get_current_branch", { repoPath: "/b" });
		const { socket, frames } = await sent(2);
		expect(FakeWebSocket.instances).toHaveLength(1);
		expect(socket.protocols).toEqual(["releash-bearer.client"]);
		expect(frames[0]).toMatchObject({
			command: "get_current_branch",
			args: { repoPath: "/a" },
		});
		socket.message({ request_id: frames[1].request_id, result: "b" });
		socket.message({ request_id: frames[0].request_id, result: "a" });
		await expect(first).resolves.toBe("a");
		await expect(second).resolves.toBe("b");
		expect(invoke).toHaveBeenCalledTimes(1);
		expect(invoke).toHaveBeenCalledWith("get_client_endpoint");
	});
	it("安全整数範囲外のworkflow応答とpushでも共有要求とterminalを継続する", async () => {
		const listener = vi.fn();
		const unlisten = await listenClient(
			"workflow-execution-changed",
			listener,
			() => {},
		);
		const output = vi.fn();
		const unstream = listenClientStream("large-value", output, vi.fn());
		const workflow = invokeClient("get_workflow_execution_state", {
			worktreePath: "/repo",
			executionId: "execution-1",
		});
		const branch = invokeClient("get_current_branch", { repoPath: "/repo" });
		const { socket, frames } = await sent(2);
		const state = fromJson(
			WorkflowExecutionViewSchema,
			clientJson(
				WorkflowExecutionViewSchema,
				{
					...JSON.parse(JSON.stringify(execution())),
					artifacts: [{ nodeName: "large", value: 0, producedAt: 1 }],
				},
				true,
			),
		);
		if (!state.artifacts?.items[0]) throw new Error("artifact");
		const deliver = (
			body: Parameters<typeof create<typeof EnvelopeSchema>>[1],
		) => {
			const bytes = toBinary(EnvelopeSchema, create(EnvelopeSchema, body));
			socket.onmessage?.({ data: bytes.buffer as ArrayBuffer });
		};
		state.artifacts.items[0].value = create(WorkflowValueSchema, {
			variant: {
				case: "signedInteger",
				value: { value: -9223372036854775808n },
			},
		});
		deliver({
			body: {
				case: "response",
				value: {
					requestId: frames[0].request_id,
					outcome: {
						case: "result",
						value: {
							command: {
								case: "getWorkflowExecutionState",
								value: { value: state },
							},
						},
					},
				},
			},
		});
		await expect(workflow).resolves.toMatchObject({
			artifacts: [{ value: Number(-9223372036854775808n) }],
		});
		state.artifacts.items[0].value = create(WorkflowValueSchema, {
			variant: {
				case: "unsignedInteger",
				value: { value: 18446744073709551615n },
			},
		});
		deliver({
			body: {
				case: "push",
				value: {
					event: {
						case: "workflowExecutionChanged",
						value: {
							worktreePath: "/repo",
							workflowExecution: state,
						},
					},
				},
			},
		});
		expect(listener).toHaveBeenLastCalledWith({
			payload: expect.objectContaining({
				workflowExecution: expect.objectContaining({
					artifacts: [
						{
							nodeName: "large",
							value: Number(18446744073709551615n),
							producedAt: 1,
						},
					],
				}),
			}),
		});
		deliver({
			body: {
				case: "stream",
				value: {
					attachmentId: "large-value",
					sequence: 1n,
					end: true,
					data: toBinary(
						TerminalEventSchema,
						create(TerminalEventSchema, {
							item: {
								case: "output",
								value: { sessionKey: "a", data: "continued", sequence: 42n },
							},
						}),
					),
				},
			},
		});
		socket.message({ request_id: frames[1].request_id, result: "main" });
		await expect(branch).resolves.toBe("main");
		expect(output).toHaveBeenCalledWith(
			expect.objectContaining({ data: "continued" }),
		);
		expect(socket.close).not.toHaveBeenCalled();
		expect(FakeWebSocket.instances).toHaveLength(1);
		unlisten();
		unstream();
	});
	it.each(["start_watching", "start_git_dir_watching"] as const)(
		"%s の成功応答を受領確認する",
		async (command) => {
			const result = invokeClient(
				command,
				command === "start_watching"
					? { path: "/repo" }
					: { repoPath: "/repo" },
			);
			const { socket, frames } = await sent();
			socket.message({ request_id: frames[0].request_id, result: 42 });
			await expect(result).resolves.toBe(42);
			expect(
				fromBinary(EnvelopeSchema, socket.send.mock.calls[1][0]).body,
			).toMatchObject({
				case: "requestAck",
				value: { requestId: frames[0].request_id },
			});
		},
	);
	it("push未読溢れでworkflowを再取得し同じ接続の要求と後続pushを維持する", async () => {
		let current = execution();
		FakeWebSocket.respond = (command) =>
			command === "resolveActiveExecutionByWorktree" ? current.id : current;
		const { result, unmount } = renderHook(() => useWorkflowState("/repo"));
		await vi.waitFor(() =>
			expect(result.current.workflowExecution?.status).toBe("running"),
		);
		const socket = FakeWebSocket.instances[0];
		current = { ...current, status: "completed" };
		await act(async () => {
			const bytes = toBinary(
				EnvelopeSchema,
				create(EnvelopeSchema, { body: { case: "pushResync", value: {} } }),
			);
			socket.onmessage?.({ data: bytes.buffer as ArrayBuffer });
		});
		await vi.waitFor(() =>
			expect(result.current.workflowExecution?.status).toBe("completed"),
		);
		await act(async () =>
			socket.message({
				status: "push",
				event: "workflow-execution-changed",
				payload: { worktreePath: "/repo", workflowExecution: execution() },
			}),
		);
		expect(result.current.workflowExecution?.status).toBe("running");
		expect(socket.close).not.toHaveBeenCalled();
		expect(FakeWebSocket.instances).toHaveLength(1);
		unmount();
	});
	it("attachment終了は対象だけ通知し別streamと保留要求を継続する", async () => {
		const closed = vi.fn(),
			otherClosed = vi.fn(),
			output = vi.fn();
		const release = listenClientStream("ended", vi.fn(), closed);
		const releaseOther = listenClientStream("other", output, otherClosed);
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const { socket, frames } = await sent();
		const end = toBinary(
			EnvelopeSchema,
			create(EnvelopeSchema, {
				body: { case: "streamClosed", value: { attachmentId: "ended" } },
			}),
		);
		socket.onmessage?.({ data: end.buffer as ArrayBuffer });
		expect(closed).toHaveBeenCalledTimes(1);
		expect(otherClosed).not.toHaveBeenCalled();
		const data = toBinary(
			TerminalEventSchema,
			create(TerminalEventSchema, {
				item: {
					case: "output",
					value: { sessionKey: "other", data: "continued", sequence: 42n },
				},
			}),
		);
		const stream = toBinary(
			EnvelopeSchema,
			create(EnvelopeSchema, {
				body: {
					case: "stream",
					value: { attachmentId: "other", sequence: 1n, data, end: true },
				},
			}),
		);
		socket.onmessage?.({ data: stream.buffer as ArrayBuffer });
		expect(output).toHaveBeenCalledWith(
			expect.objectContaining({ data: "continued" }),
		);
		socket.message({ request_id: frames[0].request_id, result: "main" });
		await expect(result).resolves.toBe("main");
		expect(socket.close).not.toHaveBeenCalled();
		release();
		releaseOther();
	});
	it("backendのerrorと切断を失敗として返す", async () => {
		const failed = invokeClient("get_current_branch", { repoPath: "/repo" });
		const failedAssertion = expect(failed).rejects.toEqual({
			code: "INVALID_REQUEST",
		});
		const { socket, frames } = await sent();
		socket.message({
			request_id: frames[0].request_id,
			error: { code: "INVALID_REQUEST" },
		});
		await failedAssertion;
		const closed = invokeClient("get_current_branch", { repoPath: "/repo" });
		const closedAssertion = expect(closed).rejects.toThrow("connection closed");
		await sent(2);
		socket.onclose?.();
		await closedAssertion;
	});
	it.each([
		["接続失敗", "flush"],
		["接続失敗", "unmount"],
		["応答前の切断", "flush"],
		["応答前の切断", "unmount"],
	])(
		"workspace保存の%s後は%sで最新状態を再送し再マウント後に復元する",
		async (failure, retry) => {
			const logError = vi.spyOn(console, "error").mockImplementation(() => {});
			FakeWebSocket.failOpen = failure === "接続失敗";
			const { result, unmount } = renderHook(() => useWorkspaceStateCache());
			const state: WorkspaceState = {
				version: 1,
				tabs: { editors: [], activeEditorPath: null },
				layout: {
					centerTab: "agent",
					activeView: "git",
					leftNavCollapsed: true,
					rightCollapsed: false,
					rightBottomCollapsed: false,
				},
			};
			act(() => {
				result.current.updateState("/repo", state);
				result.current.flushState("/repo");
			});
			if (failure === "応答前の切断") {
				const { socket } = await sent();
				await act(async () => socket.onclose?.());
			}
			await vi.waitFor(() => expect(logError).toHaveBeenCalledTimes(1));
			expect(result.current.getState("/repo")).toEqual(state);
			FakeWebSocket.failOpen = false;
			const connected = invokeClient("get_current_branch", {
				repoPath: "/repo",
			});
			const connection = await sent(1, 1);
			connection.socket.message({
				request_id: connection.frames[0].request_id,
				result: "main",
			});
			await connected;
			act(() => {
				if (retry === "unmount") unmount();
				else result.current.flushState("/repo");
			});
			const saved = await sent(2, 1);
			expect(saved.frames[1]).toMatchObject({
				command: "save_workspace_state",
				args: { worktreeName: "repo", state },
			});
			await act(async () => {
				saved.socket.message({
					request_id: saved.frames[1].request_id,
					result: null,
				});
			});
			if (retry === "flush") unmount();
			expect(saved.socket.send).toHaveBeenCalledTimes(2);
			const restored = renderHook(() => useWorkspaceStateCache());
			expect(restored.result.current.getState("/repo")).toBeUndefined();
			await act(async () => {
				const loaded = restored.result.current.loadState("/repo");
				const loading = await sent(3, 1);
				expect(loading.frames[2]).toMatchObject({
					command: "load_workspace_state",
					args: { worktreeName: "repo", worktreeRoot: "/repo" },
				});
				loading.socket.message({
					request_id: loading.frames[2].request_id,
					result: (saved.frames[1].args as { state: JsonValue }).state,
				});
				await expect(loaded).resolves.toEqual(state);
			});
			expect(restored.result.current.getState("/repo")).toEqual(state);
			restored.unmount();
			expect(invoke).toHaveBeenCalledTimes(2);
			expect(invoke).toHaveBeenCalledWith("get_client_endpoint");
		},
	);
	it("workflow pushを購読者へ届け解除後は届けない", async () => {
		const listener = vi.fn();
		const unlisten = await listenClient(
			"workflow-execution-changed",
			listener,
			vi.fn(),
		);
		const socket = FakeWebSocket.instances[0];
		const payload = {
			worktreePath: "/repo",
			workflowExecution: execution(),
		};
		socket.message({
			status: "push",
			event: "workflow-execution-changed",
			payload,
		});
		expect(listener).toHaveBeenCalledWith({ payload });
		socket.message({
			status: "push",
			event: "branch-list-sync",
			payload: null,
		});
		unlisten();
		socket.message({
			status: "push",
			event: "workflow-execution-changed",
			payload,
		});
		expect(listener).toHaveBeenCalledTimes(1);
	});
	it("接続情報取得失敗後は次回取得を再試行する", async () => {
		vi.mocked(invoke).mockResolvedValueOnce(null);
		await expect(
			invokeClient("get_current_branch", { repoPath: "/repo" }),
		).rejects.toThrow("unavailable");
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const { socket, frames } = await sent();
		socket.message({ request_id: frames[0].request_id, result: "retry" });
		await expect(result).resolves.toBe("retry");
		expect(invoke).toHaveBeenCalledTimes(2);
	});
	it("ブランチ要求は送信から10秒で解放し遅延応答を無視して同じ接続を使える", async () => {
		vi.useFakeTimers();
		FakeWebSocket.autoOpen = false;
		const rejected = vi.fn();
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		void result.catch(rejected);
		await vi.advanceTimersByTimeAsync(5_000);
		const socket = FakeWebSocket.instances[0];
		expect(socket.send).not.toHaveBeenCalled();
		socket.onopen?.();
		await vi.advanceTimersByTimeAsync(9_999);
		expect(rejected).not.toHaveBeenCalled();
		const { frames } = await sent();
		await vi.advanceTimersByTimeAsync(1);
		expect(rejected).toHaveBeenCalledExactlyOnceWith(
			new Error("Client command timed out: get_current_branch"),
		);
		expect(vi.getTimerCount()).toBe(0);
		const late = toBinary(
			EnvelopeSchema,
			create(EnvelopeSchema, {
				body: {
					case: "response",
					value: {
						requestId: frames[0].request_id,
						outcome: { case: "result", value: {} },
					},
				},
			}),
		);
		socket.onmessage?.({ data: late.buffer as ArrayBuffer });
		expect(socket.close).not.toHaveBeenCalled();
		const retry = invokeClient("get_current_branch", { repoPath: "/repo" });
		const { frames: next } = await sent(2);
		socket.message({ request_id: next[1].request_id, result: "main" });
		await expect(retry).resolves.toBe("main");
		expect(FakeWebSocket.instances).toHaveLength(1);
		expect(vi.getTimerCount()).toBe(0);
	});
	it("ブランチ再取得が無応答なら10秒後に表示を空にしてrefreshを終了する", async () => {
		vi.useFakeTimers();
		const log = vi.spyOn(console, "error").mockImplementation(() => {});
		FakeWebSocket.respond = () => "main";
		const { result, unmount } = renderHook(() => useCurrentBranch("/repo"));
		await act(() => vi.advanceTimersByTimeAsync(0));
		expect(result.current.branch).toBe("main");
		FakeWebSocket.respond = null;
		const refresh = result.current.refresh();
		await act(() => vi.advanceTimersByTimeAsync(10_000));
		expect(result.current.branch).toBeNull();
		await expect(refresh).resolves.toBeUndefined();
		expect(log).toHaveBeenCalledWith(
			"[useCurrentBranch] Failed to get branch:",
			new Error("Client command timed out: get_current_branch"),
		);
		unmount();
	});
	it.each(["error", "disconnect", "send"])(
		"ブランチ要求の%s失敗でタイマーを残さない",
		async (failure) => {
			vi.useFakeTimers();
			FakeWebSocket.autoOpen = false;
			const result = invokeClient("get_current_branch", { repoPath: "/repo" });
			const assertion = expect(result).rejects.toBeDefined();
			await vi.advanceTimersByTimeAsync(0);
			const socket = FakeWebSocket.instances[0];
			if (failure === "send")
				socket.send.mockImplementationOnce(() => {
					throw new Error("send failed");
				});
			socket.onopen?.();
			const { frames } = await sent();
			if (failure === "error")
				socket.message({
					request_id: frames[0].request_id,
					error: { code: "INVALID_REQUEST" },
				});
			else if (failure === "disconnect") socket.onclose?.();
			await assertion;
			expect(vi.getTimerCount()).toBe(0);
		},
	);
	it("長時間commandは接続中に打ち切らず応答を返す", async () => {
		vi.useFakeTimers();
		const result = invokeClient("start_workflow", {
			workflowName: "test",
			worktreePath: "/repo",
		});
		await vi.advanceTimersByTimeAsync(60_000);
		const socket = FakeWebSocket.instances[0];
		const body = fromBinary(EnvelopeSchema, socket.send.mock.calls[0][0]).body;
		if (body.case !== "request") throw new Error("request");
		socket.message({ request_id: body.value.requestId, result: "completed" });
		await expect(result).resolves.toBe("completed");
	});
	it.each(["endpoint", "unavailable", "handshake", "timeout"])(
		"初期接続失敗(%s)後も有効な購読を回復し解除済み購読は呼ばない",
		async (failure) => {
			vi.useFakeTimers();
			vi.spyOn(console, "warn").mockImplementation(() => {});
			if (failure === "endpoint")
				vi.mocked(invoke).mockRejectedValueOnce(new Error("endpoint failed"));
			else if (failure === "unavailable")
				vi.mocked(invoke).mockResolvedValueOnce(null);
			else if (failure === "timeout") FakeWebSocket.autoOpen = false;
			else FakeWebSocket.failOpen = true;
			const listener = vi.fn();
			const reconnect = vi.fn();
			const unlisten = await listenClient(
				"workflow-execution-changed",
				listener,
				reconnect,
			);
			const removedListener = vi.fn();
			const removedReconnect = vi.fn();
			const remove = await listenClient(
				"workflow-execution-changed",
				removedListener,
				removedReconnect,
			);
			remove();
			await vi.advanceTimersByTimeAsync(failure === "timeout" ? 10_000 : 0);
			expect(reconnect).not.toHaveBeenCalled();
			FakeWebSocket.failOpen = false;
			FakeWebSocket.autoOpen = true;
			await vi.advanceTimersByTimeAsync(1_000);
			expect(reconnect).toHaveBeenCalledTimes(1);
			const payload = { worktreePath: "/repo", workflowExecution: execution() };
			FakeWebSocket.instances[FakeWebSocket.instances.length - 1]?.message({
				status: "push",
				event: "workflow-execution-changed",
				payload,
			});
			expect(listener).toHaveBeenCalledExactlyOnceWith({ payload });
			expect(removedListener).not.toHaveBeenCalled();
			expect(removedReconnect).not.toHaveBeenCalled();
			unlisten();
		},
	);

	it("初期接続待ちで全購読を解除するとcallbackも再試行も残らない", async () => {
		vi.useFakeTimers();
		FakeWebSocket.autoOpen = false;
		const listener = vi.fn();
		const reconnect = vi.fn();
		const unlisten = await listenClient(
			"workflow-execution-changed",
			listener,
			reconnect,
		);
		unlisten();
		const socket = FakeWebSocket.instances[0];
		socket.onopen?.();
		socket.message({
			status: "push",
			event: "workflow-execution-changed",
			payload: { worktreePath: "/repo", workflowExecution: execution() },
		});
		socket.onclose?.();
		await vi.advanceTimersByTimeAsync(5_000);
		expect(listener).not.toHaveBeenCalled();
		expect(reconnect).not.toHaveBeenCalled();
		expect(invoke).toHaveBeenCalledTimes(1);
	});

	it("同じworktreeの画面は初期接続失敗から現在状態と後続pushを回復する", async () => {
		vi.useFakeTimers();
		vi.spyOn(console, "warn").mockImplementation(() => {});
		let attempts = 0;
		let current = { ...execution(), currentNode: "before" };
		vi.mocked(invoke).mockImplementation(async (command) => {
			if (command === "get_client_endpoint") {
				if (++attempts === 1) throw new Error("endpoint failed");
				return {
					url: "ws://127.0.0.1:123/v1/client",
					authSubprotocol: "releash-bearer.client",
				};
			}
			if (command === "resolve_active_execution_by_worktree")
				return "execution-1";
			if (command === "get_workflow_execution_state") return current;
			throw new Error(`Unexpected invoke: ${command}`);
		});
		const { result, unmount } = renderHook(() => useWorkflowState("/repo"));
		await act(async () => {
			await vi.advanceTimersByTimeAsync(0);
		});
		expect(result.current.workflowExecution).toBeNull();
		FakeWebSocket.respond = (command) =>
			command === "resolveActiveExecutionByWorktree" ? "execution-1" : current;
		current = { ...current, currentNode: "offline-update" };
		await act(async () => {
			await vi.advanceTimersByTimeAsync(1_000);
		});
		expect(result.current.workflowExecution?.currentNode).toBe(
			"offline-update",
		);
		await act(async () => {
			FakeWebSocket.instances[0].message({
				status: "push",
				event: "workflow-execution-changed",
				payload: {
					worktreePath: "/repo",
					workflowExecution: { ...current, currentNode: "latest-push" },
				},
			});
		});
		expect(result.current.workflowExecution?.currentNode).toBe("latest-push");
		unmount();
	});

	it("切断後は同じ購読を再接続し失敗しても再試行して現在状態の再取得を通知する", async () => {
		vi.useFakeTimers();
		const listener = vi.fn();
		const reconnect = vi.fn();
		const unlisten = await listenClient(
			"workflow-execution-changed",
			listener,
			reconnect,
		);
		await vi.advanceTimersByTimeAsync(0);
		expect(reconnect).toHaveBeenCalledTimes(1);
		reconnect.mockClear();
		const removedListener = vi.fn();
		const removedReconnect = vi.fn();
		const remove = await listenClient(
			"workflow-execution-changed",
			removedListener,
			removedReconnect,
		);
		remove();
		const first = FakeWebSocket.instances[0];
		first.onclose?.();
		first.onerror?.();
		vi.mocked(invoke).mockRejectedValueOnce(new Error("endpoint unavailable"));
		const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
		await vi.advanceTimersByTimeAsync(1_000);
		expect(reconnect).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1_000);
		expect(FakeWebSocket.instances).toHaveLength(2);
		expect(reconnect).toHaveBeenCalledTimes(1);
		const frame = {
			status: "push",
			event: "workflow-execution-changed",
			payload: { worktreePath: "/repo", workflowExecution: execution() },
		};
		first.message(frame);
		FakeWebSocket.instances[1].message(frame);
		expect(listener).toHaveBeenCalledTimes(1);
		expect(removedListener).not.toHaveBeenCalled();
		expect(removedReconnect).not.toHaveBeenCalled();
		FakeWebSocket.instances[1].onclose?.();
		unlisten();
		await vi.advanceTimersByTimeAsync(5_000);
		expect(FakeWebSocket.instances).toHaveLength(2);
		warn.mockRestore();
	});
	it("streamの分割ackと通常応答を1接続で多重化する", async () => {
		const output = vi.fn();
		const unlisten = listenClientStream("a", output, vi.fn());
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const { socket, frames } = await sent();
		const bytes = toBinary(
			TerminalEventSchema,
			create(TerminalEventSchema, {
				item: {
					case: "output",
					value: { sessionKey: "a", data: "日本語", sequence: 41n },
				},
			}),
		);
		const send = (sequence: bigint, data: Uint8Array, end: boolean) => {
			const frame = toBinary(
				EnvelopeSchema,
				create(EnvelopeSchema, {
					body: {
						case: "stream",
						value: { attachmentId: "a", sequence, data, end },
					},
				}),
			);
			socket.onmessage?.({ data: frame.buffer as ArrayBuffer });
		};
		send(1n, bytes.slice(0, 5), false);
		expect(output).not.toHaveBeenCalled();
		expect(
			fromBinary(EnvelopeSchema, socket.send.mock.calls[1][0]).body,
		).toMatchObject({
			case: "ack",
			value: { attachmentId: "a", sequence: 1n },
		});
		socket.message({ request_id: frames[0].request_id, result: "main" });
		await expect(result).resolves.toBe("main");
		send(2n, bytes.slice(5), true);
		expect(output).toHaveBeenCalledWith({
			type: "output",
			session_key: "a",
			data: "日本語",
			sequence: 41,
		});
		expect(socket.send).toHaveBeenCalledTimes(3);
		acknowledgeClientStream("a", 41);
		expect(
			fromBinary(EnvelopeSchema, socket.send.mock.calls[3][0]).body,
		).toMatchObject({
			case: "ack",
			value: { sequence: 0n, outputSequence: 41n },
		});
		unlisten();
	});
	it("stream上限超過は接続を閉じて保留要求を解放する", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const assertion = expect(result).rejects.toThrow("connection closed");
		const { socket } = await sent();
		const frame = toBinary(
			EnvelopeSchema,
			create(EnvelopeSchema, {
				body: {
					case: "stream",
					value: {
						attachmentId: "a",
						sequence: 1n,
						data: new Uint8Array(65536),
						end: true,
					},
				},
			}),
		);
		socket.onmessage?.({ data: frame.buffer as ArrayBuffer });
		await assertion;
		expect(socket.close).toHaveBeenCalledOnce();
	});
	it("terminal購読の解除後は切断時の再接続を止める", async () => {
		vi.useFakeTimers();
		const changed = vi.fn();
		const unlisten = onClientConnection(changed);
		await vi.advanceTimersByTimeAsync(0);
		expect(changed).toHaveBeenCalledWith(true);
		FakeWebSocket.instances[0].onclose?.();
		expect(changed).toHaveBeenLastCalledWith(false);
		unlisten();
		await vi.advanceTimersByTimeAsync(5000);
		expect(FakeWebSocket.instances).toHaveLength(1);
	});
	it.each(["initial", "disconnect"])(
		"接続listenerだけで%sから自動再接続する",
		async (failure) => {
			vi.useFakeTimers();
			vi.spyOn(console, "warn").mockImplementation(() => {});
			if (failure === "initial") FakeWebSocket.failOpen = true;
			const changed = vi.fn();
			const unlisten = onClientConnection(changed);
			try {
				await vi.advanceTimersByTimeAsync(0);
				if (failure === "disconnect") FakeWebSocket.instances[0].onclose?.();
				changed.mockClear();
				FakeWebSocket.failOpen = false;
				await vi.advanceTimersByTimeAsync(1_000);
				expect(FakeWebSocket.instances).toHaveLength(2);
				expect(changed).toHaveBeenCalledExactlyOnceWith(true);
			} finally {
				unlisten();
			}
		},
	);
	it.each(["result", "error"] as const)(
		"相関応答の%s変換が失敗しても要求をrejectする",
		async (outcome) => {
			vi.spyOn(console, "error").mockImplementation(() => {});
			const result = invokeClient("get_current_branch", { repoPath: "/repo" });
			const assertion = expect(result).rejects.toThrow();
			const { socket, frames } = await sent();
			const bytes = toBinary(
				EnvelopeSchema,
				create(EnvelopeSchema, {
					body: {
						case: "response",
						value: {
							requestId: frames[0].request_id,
							outcome: { case: outcome, value: {} },
						},
					},
				}),
			);
			socket.onmessage?.({ data: bytes.buffer as ArrayBuffer });
			await assertion;
		},
	);

	it("同じrequest_idでも異なるcommandの結果はrejectする", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const assertion = expect(result).rejects.toThrow();
		const { socket, frames } = await sent();
		const bytes = toBinary(
			EnvelopeSchema,
			create(EnvelopeSchema, {
				body: {
					case: "response",
					value: {
						requestId: frames[0].request_id,
						outcome: {
							case: "result",
							value: {
								command: {
									case: "getCrashReportingEnabled",
									value: { value: true },
								},
							},
						},
					},
				},
			}),
		);
		socket.onmessage?.({ data: bytes.buffer as ArrayBuffer });
		await assertion;
	});
});
