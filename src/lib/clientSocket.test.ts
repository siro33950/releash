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
vi.mock("monaco-editor", () => ({}));
vi.mock("@tauri-apps/plugin-autostart", () => ({
	isEnabled: vi.fn().mockResolvedValue(false),
	enable: vi.fn(),
	disable: vi.fn(),
}));

import type { JsonValue } from "@bufbuild/protobuf";
import { invoke } from "@tauri-apps/api/core";
import {
	act,
	fireEvent,
	render,
	renderHook,
	screen,
} from "@testing-library/react";
import { createElement } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ClientConnectionBanner } from "@/components/ClientConnectionBanner";
import { SettingsModal } from "@/components/panels/SettingsModal";
import { CLIENT_TRANSPORT } from "@/generated/client_transport";
import { useAutomation } from "@/hooks/useAutomation";
import { useCurrentBranch } from "@/hooks/useCurrentBranch";
import { useGitEventRefresh } from "@/hooks/useGitEventRefresh";
import { useRepoList } from "@/hooks/useRepoList";
import { useReviewFileView } from "@/hooks/useReviewFileView";
import { useWorkflowState } from "@/hooks/useWorkflowState";
import { useWorktreeList } from "@/hooks/useWorktreeList";
import { DEFAULT_SETTINGS } from "@/types/settings";
import type { WorkflowExecution } from "@/types/workflow";
import { subscribeAgentSessionChanged } from "./agentSessionEvents";
import { clientJson } from "./clientJson";
import {
	acknowledgeClientStream,
	dismissClientOperation,
	getClientStatus,
	invokeClient,
	listenClient,
	listenClientStream,
	onClientConnection,
	retryClientOperation,
	watchClient,
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
	static heartbeatResponds = true;
	static instanceId = "backend-1";
	static autoOpen = true;
	static nextDecision: {
		state: string;
		operationId?: string;
		fingerprint?: string;
		orderingTarget?: string;
	} | null = null;
	static hello: Record<string, unknown> = {};
	proposals = vi.fn();
	queries = new Map<string, string>();
	static respond: ((command: string) => unknown) | null = null;
	onopen: (() => void) | null = null;
	onerror: (() => void) | null = null;
	onclose: (() => void) | null = null;
	onmessage: ((event: { data: ArrayBuffer }) => void) | null = null;
	send = (bytes: Uint8Array) => {
		const body = fromBinary(EnvelopeSchema, bytes).body;
		if (body.case === "hello") {
			this.control({
				hello: { instanceId: FakeWebSocket.instanceId, ...FakeWebSocket.hello },
			} as JsonValue);
			return;
		}
		if (body.case === "heartbeat") {
			if (FakeWebSocket.heartbeatResponds)
				this.control({ heartbeat: { nonce: body.value.nonce } });
			return;
		}
		if (body.case === "operationQuery")
			this.queries.set(body.value.requestId, body.value.queryId);
		if (body.case === "requestAck" && body.value.confirmWatch) {
			this.sendFrame(bytes);
			this.control({
				operationStatus: {
					requestId: body.value.requestId,
					state: "watch_active",
				},
			});
			return;
		}
		if (body.case === "operationQuery" && !body.value.sent) {
			this.proposals(body.value);
			const decision = FakeWebSocket.nextDecision ?? { state: "ready" };
			FakeWebSocket.nextDecision = null;
			this.control({
				operationStatus: { requestId: body.value.requestId, ...decision },
			});
			return;
		}
		this.sendFrame(bytes);
	};
	control(envelope: JsonValue) {
		const status = (
			envelope as { operationStatus?: { requestId: string; queryId?: string } }
		).operationStatus;
		if (status && status.queryId === undefined)
			status.queryId = this.queries.get(status.requestId) ?? "";
		const bytes = toBinary(EnvelopeSchema, fromJson(EnvelopeSchema, envelope));
		this.onmessage?.({ data: bytes.buffer as ArrayBuffer });
	}
	sendFrame = vi.fn((bytes: Uint8Array) => {
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
			const request = this.sendFrame.mock.calls
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
	const requests = () =>
		(FakeWebSocket.instances[socketIndex]?.sendFrame.mock.calls ?? [])
			.map(([frame]) => fromBinary(EnvelopeSchema, frame).body)
			.filter((body) => body.case === "request");
	await vi.waitFor(() => expect(requests()).toHaveLength(count));
	return {
		socket: FakeWebSocket.instances[socketIndex],
		frames: requests().map((body) => {
			if (body.case !== "request") throw new Error("request");
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

function observeResult(result: Promise<unknown>) {
	const resolved = vi.fn();
	const rejected = vi.fn();
	void result.then(resolved, rejected);
	return { resolved, rejected };
}

describe("clientSocket", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		FakeWebSocket.instances = [];
		FakeWebSocket.nextDecision = null;
		FakeWebSocket.hello = {};
		FakeWebSocket.respond = null;
		FakeWebSocket.failOpen = false;
		FakeWebSocket.heartbeatResponds = true;
		FakeWebSocket.instanceId = "backend-1";
		FakeWebSocket.autoOpen = true;
		vi.stubGlobal("WebSocket", FakeWebSocket);
		vi.mocked(invoke).mockResolvedValue({
			url: "ws://127.0.0.1:123/v1/client",
			authSubprotocol: "releash-bearer.client",
		});
	});
	afterEach(() => {
		for (const socket of FakeWebSocket.instances) socket.onclose?.();
		window.dispatchEvent(new Event("pagehide"));
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
			await expect(result).resolves.toBe(-1);
			expect(
				fromBinary(EnvelopeSchema, socket.sendFrame.mock.calls[1][0]).body,
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
	it("backendの確定エラーは失敗として返す", async () => {
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const assertion = expect(result).rejects.toEqual({
			code: "INVALID_REQUEST",
		});
		const { socket, frames } = await sent();
		socket.message({
			request_id: frames[0].request_id,
			error: { code: "INVALID_REQUEST" },
		});
		await assertion;
	});
	it("完了後の切断では変更を再送せず元の操作の結果を照会する", async () => {
		vi.useFakeTimers();
		const result = invokeClient("start_workflow", {
			workflowName: "test",
			worktreePath: "/repo",
		});
		const { socket, frames } = await sent();
		socket.onclose?.();
		await vi.advanceTimersByTimeAsync(1_000);
		const recovered = FakeWebSocket.instances[1];
		const query = fromBinary(
			EnvelopeSchema,
			recovered.sendFrame.mock.calls[0][0],
		).body;
		expect(query.case).toBe("operationQuery");
		expect(getClientStatus().operations).toContainEqual({
			id: frames[0].request_id,
			command: "start_workflow",
			state: "unknown",
			expired: false,
		});
		recovered.control({
			response: {
				requestId: frames[0].request_id,
				result: { startWorkflow: { value: "execution" } },
			},
		});
		await expect(result).resolves.toBe("execution");
		expect(getClientStatus().operations).toEqual([]);
	});
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
	it("未送信の変更要求は期限内の接続回復で一度送る", async () => {
		vi.useFakeTimers();
		vi.mocked(invoke).mockResolvedValueOnce(null);
		const result = invokeClient("update_crash_reporting", { enabled: true });
		await vi.advanceTimersByTimeAsync(1_000);
		const { socket, frames } = await sent();
		socket.message({ request_id: frames[0].request_id, result: null });
		await expect(result).resolves.toBeNull();
	});
	it("未送信要求は期限後に接続しても送らない", async () => {
		vi.useFakeTimers();
		FakeWebSocket.failOpen = true;
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const rejected = expect(result).rejects.toMatchObject({
			state: "not_sent",
		});
		await vi.advanceTimersByTimeAsync(10_000);
		await rejected;
		FakeWebSocket.failOpen = false;
		await vi.advanceTimersByTimeAsync(2_000);
		expect(
			FakeWebSocket.instances.flatMap((socket) => socket.sendFrame.mock.calls),
		).toEqual([]);
	});
	it.each([null, new Error("endpoint failed")])(
		"未送信の変更を期限後に未実行と表示し回復しても送らない: %s",
		async (endpoint) => {
			vi.useFakeTimers();
			if (endpoint instanceof Error)
				vi.mocked(invoke).mockRejectedValue(endpoint);
			else vi.mocked(invoke).mockResolvedValue(endpoint);
			const view = render(createElement(ClientConnectionBanner));
			const result = invokeClient("add_repo_path", { path: "/repo" });
			const rejected = expect(result).rejects.toMatchObject({
				state: "not_sent",
			});
			await act(() => vi.advanceTimersByTimeAsync(1_000));
			expect(screen.getByRole("status")).toHaveTextContent(
				"接続情報を取得できません",
			);
			expect(screen.getByRole("status")).toHaveTextContent("未送信");
			await act(() => vi.advanceTimersByTimeAsync(29_000));
			await rejected;
			expect(screen.getByRole("status")).toHaveTextContent(
				"add_repo_path: 要求は送信されていません（未実行）。",
			);
			const [operation] = getClientStatus().operations;
			vi.mocked(invoke).mockResolvedValue({
				url: "ws://127.0.0.1:123/v1/client",
				authSubprotocol: "client",
			});
			await act(() => vi.advanceTimersByTimeAsync(1_000));
			expect(getClientStatus().connected).toBe(true);
			expect(screen.getByRole("status")).toHaveTextContent("未実行");
			expect(
				FakeWebSocket.instances.flatMap(
					(socket) => socket.sendFrame.mock.calls,
				),
			).toEqual([]);
			act(() => dismissClientOperation(operation.id));
			expect(screen.queryByRole("status")).not.toBeInTheDocument();
			view.unmount();
		},
	);
	it.each([true, false])(
		"readの期限切れと切断の順序によらず管理枠を解放する: %s",
		{ timeout: 30_000 },
		async (expireFirst) => {
			vi.useFakeTimers();
			const off = onClientConnection(vi.fn());
			const results = Array.from(
				{ length: CLIENT_TRANSPORT.maxPending },
				(_, i) =>
					invokeClient("get_current_branch", { repoPath: `/repo/${i}` }),
			);
			const rejected = Promise.all(
				results.map((result) =>
					expect(result).rejects.toMatchObject({ state: "unknown" }),
				),
			);
			const { socket } = await sent(CLIENT_TRANSPORT.maxPending);
			if (expireFirst) await vi.advanceTimersByTimeAsync(10_000);
			FakeWebSocket.failOpen = true;
			socket.onclose?.();
			await vi.advanceTimersByTimeAsync(10_000);
			await rejected;
			FakeWebSocket.failOpen = false;
			await vi.advanceTimersByTimeAsync(1_000);
			const next = invokeClient("get_current_branch", { repoPath: "/new" });
			const recovered = await sent(1, FakeWebSocket.instances.length - 1);
			recovered.socket.message({
				request_id: recovered.frames[0].request_id,
				result: "main",
			});
			await expect(next).resolves.toBe("main");
			expect(getClientStatus().operations).toEqual([]);
			off();
		},
	);
	it("同名の冪等commandでも別対象は先行要求の期限超過に巻き込まれない", async () => {
		vi.useFakeTimers();
		const first = invokeClient("add_repo_path", { path: "/a" });
		const observed = observeResult(first);
		const initial = await sent();
		await vi.advanceTimersByTimeAsync(30_000);
		expect(observed.rejected).not.toHaveBeenCalled();
		expect(observed.resolved).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ state: "unknown" }),
		);
		const other = invokeClient("add_repo_path", { path: "/b" });
		const { socket, frames } = await sent(2);
		socket.message({ request_id: frames[1].request_id, result: true });
		await expect(other).resolves.toBe(true);
		FakeWebSocket.nextDecision = {
			state: "bound",
			operationId: initial.frames[0].request_id,
		};
		const retry = invokeClient("add_repo_path", { path: "/a" });
		expect(socket.close).not.toHaveBeenCalled();
		socket.message({ request_id: frames[0].request_id, result: true });
		await expect(first).resolves.toBe(true);
		await expect(retry).resolves.toBe(true);
	});
	it("個別要求の期限後も接続を維持し遅延応答で現在状態を再取得する", async () => {
		vi.useFakeTimers();
		const refresh = vi.fn();
		const off = await listenClient("branch-list-sync", () => {}, refresh);
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const rejected = expect(result).rejects.toMatchObject({ state: "unknown" });
		const { socket, frames } = await sent();
		await vi.advanceTimersByTimeAsync(10_000);
		await rejected;
		expect(socket.close).not.toHaveBeenCalled();
		refresh.mockClear();
		socket.message({ request_id: frames[0].request_id, result: "late" });
		expect(refresh).toHaveBeenCalledOnce();
		const next = invokeClient("get_current_branch", { repoPath: "/repo" });
		const second = await sent(2);
		socket.message({ request_id: second.frames[1].request_id, result: "main" });
		await expect(next).resolves.toBe("main");
		off();
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
			expect.objectContaining({ state: "unknown" }),
		);
		unmount();
	});
	it("送信APIが送信前に失敗した要求はbackendへ送らない", async () => {
		FakeWebSocket.autoOpen = false;
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const rejected = expect(result).rejects.toThrow("send failed");
		await vi.waitFor(() => expect(FakeWebSocket.instances).toHaveLength(1));
		const socket = FakeWebSocket.instances[0];
		socket.sendFrame.mockImplementationOnce(() => {
			throw new Error("send failed");
		});
		socket.onopen?.();
		await rejected;
	});
	it("長時間commandは接続中に打ち切らず応答を返す", async () => {
		vi.useFakeTimers();
		const result = invokeClient("start_workflow", {
			workflowName: "test",
			worktreePath: "/repo",
		});
		await vi.advanceTimersByTimeAsync(60_000);
		const socket = FakeWebSocket.instances[0];
		const body = fromBinary(
			EnvelopeSchema,
			socket.sendFrame.mock.calls[0][0],
		).body;
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
			expect(reconnect).not.toHaveBeenCalled();
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
		expect(reconnect).not.toHaveBeenCalled();
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
	it("review画像のWS応答をdata URLのまま表示へ渡す", async () => {
		const { result, unmount } = renderHook(() =>
			useReviewFileView("/repo", "画像.png", "head", "changes", 0, null),
		);
		const { socket, frames } = await sent();
		const reference =
			"blob?worktree=%2Frepo&path=画像.png&side=modified&section=changes&base=head&version=1";
		await act(async () =>
			socket.message({
				request_id: frames[0].request_id,
				result: {
					kind: "image",
					version: 1,
					stale: false,
					fileId: "image",
					path: "画像.png",
					originalUrl: null,
					modifiedUrl: reference,
					mime: "image/png",
				},
			}),
		);
		const request = await sent(2);
		expect(request.frames[1]).toMatchObject({
			command: "get_review_blob",
			args: { reference },
		});
		const dataUrl =
			"data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII=";
		await act(async () =>
			socket.message({
				request_id: request.frames[1].request_id,
				result: dataUrl,
			}),
		);
		expect(result.current.imageDiff).toEqual({
			originalUrl: null,
			modifiedUrl: dataUrl,
			loading: false,
		});
		expect(result.current.error).toBeNull();
		unmount();
	});
	it("個別変更要求の期限超過中もterminal入出力と別要求を継続する", async () => {
		vi.useFakeTimers();
		const change = invokeClient("delete_review_thread", {
			worktreeName: "/repo",
			threadId: "t",
		});
		const observed = observeResult(change);
		const { socket, frames } = await sent();
		const output = vi.fn();
		const off = listenClientStream("a", output, vi.fn());
		await vi.advanceTimersByTimeAsync(30_000);
		expect(observed.rejected).not.toHaveBeenCalled();
		expect(observed.resolved).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ state: "unknown" }),
		);
		const input = invokeClient("write_terminal_surface", {
			owner: { kind: "workspace", workspacePath: "/repo" },
			attachmentId: "a",
			sequence: 0,
			data: "echo hi\n",
		});
		const other = invokeClient("get_current_branch", { repoPath: "/repo" });
		const current = await sent(3);
		socket.control({
			stream: {
				attachmentId: "a",
				sequence: "1",
				end: true,
				data: btoa(
					String.fromCharCode(
						...toBinary(
							TerminalEventSchema,
							create(TerminalEventSchema, {
								item: {
									case: "output",
									value: { sessionKey: "a", data: "hi", sequence: 42n },
								},
							}),
						),
					),
				),
			},
		});
		expect(output).toHaveBeenCalledWith({
			type: "output",
			session_key: "a",
			data: "hi",
			sequence: 42,
		});
		acknowledgeClientStream("a", 42);
		for (const frame of current.frames.slice(1))
			socket.message({
				request_id: frame.request_id,
				result: frame.command === "get_current_branch" ? "main" : null,
			});
		await expect(input).resolves.toBeNull();
		await expect(other).resolves.toBe("main");
		expect(socket.close).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ id: frames[0].request_id, state: "unknown" }),
		);
		expect(
			socket.sendFrame.mock.calls.map(
				([bytes]) => fromBinary(EnvelopeSchema, bytes).body,
			),
		).toContainEqual(
			expect.objectContaining({
				case: "ack",
				value: expect.objectContaining({ outputSequence: 42n }),
			}),
		);
		socket.message({ request_id: frames[0].request_id, result: null });
		await expect(change).resolves.toBeNull();
		expect(getClientStatus().operations).toEqual([]);
		off();
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
			fromBinary(EnvelopeSchema, socket.sendFrame.mock.calls[1][0]).body,
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
		expect(socket.sendFrame).toHaveBeenCalledTimes(3);
		acknowledgeClientStream("a", 41);
		expect(
			fromBinary(EnvelopeSchema, socket.sendFrame.mock.calls[3][0]).body,
		).toMatchObject({
			case: "ack",
			value: { sequence: 0n, outputSequence: 41n },
		});
		unlisten();
	});
	it("stream上限超過は接続を閉じて保留要求を解放する", async () => {
		vi.spyOn(console, "error").mockImplementation(() => {});
		const result = invokeClient("get_current_branch", { repoPath: "/repo" });
		const assertion = expect(result).rejects.toMatchObject({
			state: "unknown",
		});
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

describe("通信停止と操作結果の復旧", () => {
	beforeEach(() => {
		vi.useFakeTimers();
		vi.clearAllMocks();
		FakeWebSocket.instances = [];
		FakeWebSocket.nextDecision = null;
		FakeWebSocket.hello = {};
		FakeWebSocket.respond = null;
		FakeWebSocket.failOpen = false;
		FakeWebSocket.autoOpen = true;
		FakeWebSocket.heartbeatResponds = true;
		FakeWebSocket.instanceId = "backend-1";
		vi.stubGlobal("WebSocket", FakeWebSocket);
		vi.mocked(invoke).mockResolvedValue({
			url: "ws://127.0.0.1:123/v1/client",
			authSubprotocol: "releash-bearer.client",
		});
	});
	afterEach(() => {
		window.dispatchEvent(new Event("pagehide"));
		vi.unstubAllGlobals();
		vi.useRealTimers();
	});
	it("pushのない待機状態では生存確認を続け接続を切らない", async () => {
		const off = onClientConnection(vi.fn());
		await vi.advanceTimersByTimeAsync(30_000);
		expect(FakeWebSocket.instances).toHaveLength(1);
		expect(FakeWebSocket.instances[0].close).not.toHaveBeenCalled();
		expect(getClientStatus().connected).toBe(true);
		off();
	});
	it("接続を閉じない通信停止を8秒で検知し表示して再接続する", async () => {
		const off = onClientConnection(vi.fn());
		await vi.advanceTimersByTimeAsync(0);
		FakeWebSocket.heartbeatResponds = false;
		await vi.advanceTimersByTimeAsync(8_000);
		expect(getClientStatus()).toMatchObject({
			connected: false,
			message: expect.stringContaining("通信状態を確認できません"),
		});
		expect(FakeWebSocket.instances[0].close).toHaveBeenCalledOnce();
		FakeWebSocket.heartbeatResponds = true;
		await vi.advanceTimersByTimeAsync(1_000);
		expect(getClientStatus()).toMatchObject({ connected: true, message: null });
		off();
	});
	it("スリープ復帰後は古い応答期限を使わず再確認する", async () => {
		const off = onClientConnection(vi.fn());
		await vi.advanceTimersByTimeAsync(0);
		FakeWebSocket.heartbeatResponds = false;
		await vi.advanceTimersByTimeAsync(5_000);
		vi.setSystemTime(Date.now() + 60_000);
		FakeWebSocket.heartbeatResponds = true;
		await vi.advanceTimersByTimeAsync(1_000);
		expect(FakeWebSocket.instances[0].close).not.toHaveBeenCalled();
		expect(getClientStatus().connected).toBe(true);
		off();
	});
	it("変更要求の期限超過と利用者の再試行でも変更を重複送信せず遅延結果で復旧する", async () => {
		const args = {
			worktreeName: "repo",
			threadId: "thread",
			content: "comment",
		};
		const result = invokeClient("append_review_comment", args);
		const observed = observeResult(result);
		const { socket, frames } = await sent();
		await vi.advanceTimersByTimeAsync(30_000);
		expect(observed.rejected).not.toHaveBeenCalled();
		expect(observed.resolved).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ state: "unknown" }),
		);
		expect(socket.close).not.toHaveBeenCalled();
		FakeWebSocket.nextDecision = {
			state: "bound",
			operationId: frames[0].request_id,
		};
		const retry = invokeClient("append_review_comment", args);
		const retried = observeResult(retry);
		retryClientOperation(frames[0].request_id);
		expect(
			socket.sendFrame.mock.calls.filter(
				([bytes]) => fromBinary(EnvelopeSchema, bytes).body.case === "request",
			),
		).toHaveLength(1);
		const other = invokeClient("get_current_branch", { repoPath: "/repo" });
		const current = await sent(2);
		socket.message({
			request_id: current.frames[1].request_id,
			result: "main",
		});
		await expect(other).resolves.toBe("main");
		socket.message({
			request_id: frames[0].request_id,
			error: { code: "CONFIRMED_ERROR" },
		});
		await expect(result).rejects.toMatchObject({ code: "CONFIRMED_ERROR" });
		await expect(retry).rejects.toMatchObject({ code: "CONFIRMED_ERROR" });
		expect(retried.rejected).toHaveBeenCalledOnce();
		expect(getClientStatus().operations).toEqual([]);
	});
	it("backend再起動で確定できない変更は結果不明を保持し再実行しない", async () => {
		const args = { workflowName: "test", worktreePath: "/repo" };
		const result = invokeClient("start_workflow", args);
		const observed = observeResult(result);
		const { socket, frames } = await sent();
		socket.onclose?.();
		FakeWebSocket.instanceId = "backend-2";
		await vi.advanceTimersByTimeAsync(1_000);
		const recovered = FakeWebSocket.instances[1];
		recovered.control({
			operationStatus: { requestId: frames[0].request_id, state: "unknown" },
		});
		FakeWebSocket.nextDecision = {
			state: "bound",
			operationId: frames[0].request_id,
		};
		const retried = observeResult(invokeClient("start_workflow", args));
		retryClientOperation(frames[0].request_id);
		expect(
			recovered.sendFrame.mock.calls.every(
				([bytes]) =>
					fromBinary(EnvelopeSchema, bytes).body.case === "operationQuery",
			),
		).toBe(true);
		await vi.advanceTimersByTimeAsync(119_000);
		expect(observed.rejected).not.toHaveBeenCalled();
		expect(observed.resolved).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ state: "unknown" }),
		);
		expect(getClientStatus().operations[0].state).toBe("unknown");
		expect(retried.resolved).not.toHaveBeenCalled();
		expect(retried.rejected).not.toHaveBeenCalled();
	});
	it("未受領の監視開始は再接続後に開始し直す", async () => {
		const watch = invokeClient("start_watching", { path: "/repo" });
		const { socket, frames } = await sent();
		socket.onclose?.();
		await vi.advanceTimersByTimeAsync(1_000);
		FakeWebSocket.instances[1].control({
			operationStatus: { requestId: frames[0].request_id, state: "ready" },
		});
		const recovered = await sent(1, 1);
		recovered.socket.message({
			request_id: recovered.frames[0].request_id,
			result: 42,
		});
		await expect(watch).resolves.toBe(-1);
	});
	it("読取要求の期限超過後も新しい読取を妨げない", async () => {
		const args = { repoPath: "/repo" };
		const first = invokeClient("get_current_branch", args);
		const expired = expect(first).rejects.toMatchObject({ state: "unknown" });
		const { socket } = await sent();
		await vi.advanceTimersByTimeAsync(10_000);
		await expired;
		const next = invokeClient("get_current_branch", args);
		const { frames } = await sent(2);
		socket.message({ request_id: frames[1].request_id, result: "next" });
		await expect(next).resolves.toBe("next");
		expect(socket.close).not.toHaveBeenCalled();
	});
	it("冪等な変更の復旧を後続の変更より先に確定する", async () => {
		const first = invokeClient("update_external_editor", { editor: "code" });
		const initial = await sent();
		initial.socket.onclose?.();
		const next = invokeClient("update_external_editor", { editor: "vim" });
		FakeWebSocket.nextDecision = { state: "blocked" };
		FakeWebSocket.instanceId = "backend-2";
		await vi.advanceTimersByTimeAsync(1_000);
		const recovered = FakeWebSocket.instances[1];
		expect(
			recovered.sendFrame.mock.calls.every(
				([bytes]) =>
					fromBinary(EnvelopeSchema, bytes).body.case === "operationQuery",
			),
		).toBe(true);
		recovered.control({
			operationStatus: {
				requestId: initial.frames[0].request_id,
				state: "ready",
			},
		});
		const replay = await sent(1, 1);
		expect(replay.frames[0]).toEqual(initial.frames[0]);
		recovered.message({
			request_id: replay.frames[0].request_id,
			result: null,
		});
		await first;
		const later = await sent(2, 1);
		expect(later.frames[1].args).toEqual({ editor: "vim" });
		recovered.message({ request_id: later.frames[1].request_id, result: null });
		await next;
	});
	it("callerRequestIdを作り直す利用者の再試行も元の操作へ対応付ける", async () => {
		const args = {
			workspaceIdentity: "/repo",
			worktreePath: "/repo",
			provider: "claude",
			rows: 24,
			cols: 80,
			callerRequestId: "original-attempt",
		};
		const first = invokeClient("create_agent_session", args);
		const { socket, frames } = await sent();
		socket.onclose?.();
		await vi.advanceTimersByTimeAsync(1_000);
		FakeWebSocket.nextDecision = {
			state: "bound",
			operationId: frames[0].request_id,
		};
		const retry = invokeClient("create_agent_session", {
			...args,
			callerRequestId: "new-attempt",
		});
		const recovered = FakeWebSocket.instances[1];
		recovered.control({
			operationStatus: { requestId: frames[0].request_id, state: "ready" },
		});
		const replay = await sent(1, 1);
		expect(replay.frames[0]).toEqual(frames[0]);
		recovered.message({ request_id: frames[0].request_id, result: "session" });
		await expect(first).resolves.toBe("session");
		await expect(retry).resolves.toBe("session");
	});
	it.each([true, false])(
		"監視の受領確認がbackendへ届いたかを再接続時に照会する: %s",
		async (acknowledged) => {
			const watch = invokeClient("start_watching", { path: "/repo" });
			const initial = await sent();
			initial.socket.message({
				request_id: initial.frames[0].request_id,
				result: 42,
			});
			await expect(watch).resolves.toBe(-1);
			const listener = vi.fn();
			const off = await listenClient("file-change", listener);
			initial.socket.onclose?.();
			await vi.advanceTimersByTimeAsync(1_000);
			FakeWebSocket.instances[1].onclose?.();
			await vi.advanceTimersByTimeAsync(1_000);
			const recovered = FakeWebSocket.instances[2];
			const originalId = initial.frames[0].request_id;
			expect(
				fromBinary(EnvelopeSchema, recovered.sendFrame.mock.calls[0][0]).body,
			).toMatchObject({
				case: "operationQuery",
				value: { requestId: originalId },
			});
			if (acknowledged) {
				recovered.control({
					response: {
						requestId: originalId,
						result: { startWatching: { value: "42" } },
					},
				});
			} else {
				recovered.control({
					operationStatus: { requestId: originalId, state: "ready" },
				});
				await sent(1, 2);
				recovered.message({ request_id: originalId, result: 43 });
			}
			await vi.advanceTimersByTimeAsync(0);
			recovered.message({
				status: "push",
				event: "file-change",
				payload: {
					watcher_id: acknowledged ? 42 : 43,
					path: "/repo/a",
					kind: "modify",
				},
			});
			expect(listener).toHaveBeenCalledWith({
				payload: expect.objectContaining({ watcher_id: -1 }),
			});
			const stop = invokeClient("stop_watching", { watcherId: -1 });
			const sentStop = await sent(acknowledged ? 1 : 2, 2);
			expect(sentStop.frames[sentStop.frames.length - 1]?.args).toEqual({
				watcherId: acknowledged ? 42 : 43,
			});
			recovered.message({
				request_id: sentStop.frames[sentStop.frames.length - 1]?.request_id,
				result: null,
			});
			await stop;
			off();
		},
	);
	it("backend再起動前の監視停止を新しい世代の同じ数値IDへ適用しない", async () => {
		const watch = invokeClient("start_watching", { path: "/repo" });
		const initial = await sent();
		initial.socket.message({
			request_id: initial.frames[0].request_id,
			result: 42,
		});
		await watch;
		initial.socket.onclose?.();
		const stop = invokeClient("stop_watching", { watcherId: -1 });
		const observed = observeResult(stop);
		FakeWebSocket.instanceId = "backend-2";
		await vi.advanceTimersByTimeAsync(1_000);
		const recovered = await sent(1, 1);
		const body = fromBinary(
			EnvelopeSchema,
			recovered.socket.sendFrame.mock.calls[0][0],
		).body;
		expect(body).toMatchObject({
			case: "request",
			value: { instanceId: "backend-1", recover: false },
		});
		recovered.socket.control({
			operationStatus: {
				requestId: recovered.frames[0].request_id,
				state: "unknown",
			},
		});
		retryClientOperation(recovered.frames[0].request_id);
		expect(
			recovered.socket.sendFrame.mock.calls.filter(
				([bytes]) => fromBinary(EnvelopeSchema, bytes).body.case === "request",
			),
		).toHaveLength(1);
		await vi.advanceTimersByTimeAsync(30_000);
		expect(observed.rejected).not.toHaveBeenCalled();
		expect(observed.resolved).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ state: "unknown" }),
		);
	});

	it.each(["invoke", "dispose"])(
		"未送信の監視停止を別世代が結果不明と判定した後も未実行へ戻さない: %s",
		async (entry) => {
			const onError = vi.fn();
			const onReady = vi.fn();
			const dispose = watchClient(
				"start_watching",
				{ path: "/repo" },
				onReady,
				onError,
			);
			const initial = await sent();
			initial.socket.message({
				request_id: initial.frames[0].request_id,
				result: 42,
			});
			await vi.advanceTimersByTimeAsync(0);
			expect(onReady).toHaveBeenCalledWith(-1);
			initial.socket.onclose?.();
			const onUncertain = vi.fn();
			const observed =
				entry === "invoke"
					? observeResult(
							invokeClient("stop_watching", { watcherId: -1 }, { onUncertain }),
						)
					: null;
			if (entry === "dispose") dispose();
			FakeWebSocket.instanceId = "backend-2";
			FakeWebSocket.nextDecision = { state: "unknown" };
			await vi.advanceTimersByTimeAsync(1_000);
			const recovered = FakeWebSocket.instances[1];
			const proposal = recovered.proposals.mock.calls[0][0];
			expect(proposal).toMatchObject({
				instanceId: "backend-1",
				sent: false,
				request: {
					command: { case: "stopWatching", value: { watcherId: 42n } },
				},
			});
			const requestId = proposal.requestId;
			const view = render(createElement(ClientConnectionBanner));
			expect(screen.getByRole("status")).toHaveTextContent(
				"操作結果を確認できません",
			);
			expect(
				screen.queryByText(/接続の回復を待っています|未実行/),
			).not.toBeInTheDocument();
			if (entry === "invoke")
				expect(onUncertain).toHaveBeenCalledWith(
					expect.objectContaining({ requestId, state: "unknown" }),
				);
			await act(() => vi.advanceTimersByTimeAsync(30_000));
			expect(getClientStatus().operations).toEqual([
				{
					id: requestId,
					command: "stop_watching",
					state: "unknown",
					expired: true,
				},
			]);
			expect(
				screen.queryByText(/接続の回復を待っています|未実行/),
			).not.toBeInTheDocument();
			fireEvent.click(
				screen.getByRole("button", { name: "元の操作の結果を確認" }),
			);
			const frames = recovered.sendFrame.mock.calls.map(
				([bytes]) => fromBinary(EnvelopeSchema, bytes).body,
			);
			expect(frames.every((body) => body.case === "operationQuery")).toBe(true);
			expect(frames[frames.length - 1]).toMatchObject({
				case: "operationQuery",
				value: {
					requestId,
					instanceId: "backend-1",
					sent: true,
					request: {
						requestId,
						instanceId: "backend-1",
						recover: true,
						userRetry: true,
					},
				},
			});
			act(() =>
				recovered.control({ operationStatus: { requestId, state: "unknown" } }),
			);
			expect(observed?.rejected ?? onError).not.toHaveBeenCalled();
			if (observed) expect(observed.resolved).not.toHaveBeenCalled();
			view.unmount();
		},
	);

	it.each([
		["retry_workspace_node", { worktreePath: "/repo", nodeId: "node" }],
		["update_external_editor", { editor: "code" }],
		["set_releash_base", { repoPath: "/repo", base: "main" }],
		[
			"save_notion_config",
			{
				repoPath: "/repo",
				apiToken: "token",
				databaseId: "db",
				propertyMapping: {
					title: "Name",
					labels: [],
					branch_name: "",
					branch_prefix: "",
				},
			},
		],
		["delete_notion_config", { repoPath: "/repo" }],
		[
			"update_provider_executable",
			{ provider: "claude", executable: "/custom/claude" },
		],
		["reset_provider_executable", { provider: "claude" }],
		["refresh_provider_availability", undefined],
	] as const)(
		"%sは期限超過と切断を通知し元の結果を重複なく受け取る",
		async (command, args) => {
			const onUncertain = vi.fn();
			const operation = invokeClient(command, args, { onUncertain });
			const observed = observeResult(operation);
			const initial = await sent();
			const requestId = initial.frames[0].request_id;
			await vi.advanceTimersByTimeAsync(
				CLIENT_TRANSPORT.commands[command].deadlineMs,
			);
			expect(onUncertain).toHaveBeenCalledWith(
				expect.objectContaining({ requestId, state: "unknown" }),
			);
			expect(initial.socket.close).not.toHaveBeenCalled();
			expect(observed.rejected).not.toHaveBeenCalled();
			onUncertain.mockClear();
			initial.socket.onclose?.();
			expect(onUncertain).toHaveBeenCalledWith(
				expect.objectContaining({ requestId, state: "unknown" }),
			);
			await vi.advanceTimersByTimeAsync(1_000);
			const recovered = FakeWebSocket.instances[1];
			retryClientOperation(requestId);
			expect(
				recovered.sendFrame.mock.calls.every(
					([bytes]) =>
						fromBinary(EnvelopeSchema, bytes).body.case === "operationQuery",
				),
			).toBe(true);
			const resultField = CommandResultSchema.fields.find(
				(field) => field.name === command,
			);
			if (!resultField?.message) throw new Error("Missing result field");
			recovered.control({
				response: {
					requestId,
					result: {
						[resultField.jsonName]: command.includes("provider")
							? clientJson(resultField.message, { providers: [] }, true)
							: {},
					},
				},
			});
			await operation;
			expect(observed.resolved).toHaveBeenCalledTimes(1);
			expect(observed.rejected).not.toHaveBeenCalled();
			expect(getClientStatus().operations).toEqual([]);
		},
	);
	it("application quitの再起動後の照会は既存journalのrequest_idを保持する", async () => {
		const args = {
			request: {
				request_id: "original-quit",
				intent: { type: "exit" as const, code: 0 },
			},
		};
		const result = invokeClient("request_application_quit", args);
		const confirmed = expect(result).rejects.toMatchObject({
			code: "CONFIRMED",
		});
		const initial = await sent();
		initial.socket.onclose?.();
		FakeWebSocket.instanceId = "backend-2";
		await vi.advanceTimersByTimeAsync(1_000);
		FakeWebSocket.nextDecision = {
			state: "bound",
			operationId: initial.frames[0].request_id,
		};
		const retry = invokeClient("request_application_quit", {
			request: { ...args.request, request_id: "retry-quit" },
		});
		const retried = expect(retry).rejects.toMatchObject({ code: "CONFIRMED" });
		const recovered = FakeWebSocket.instances[1];
		recovered.control({
			operationStatus: {
				requestId: initial.frames[0].request_id,
				state: "ready",
			},
		});
		const replay = await sent(1, 1);
		expect(replay.frames[0]).toEqual(initial.frames[0]);
		recovered.message({
			request_id: replay.frames[0].request_id,
			error: { code: "CONFIRMED" },
		});
		await confirmed;
		await retried;
	});
	it("応答未受領の変更が保持上限に達しても新規変更の受付をRustへ委ねる", {
		timeout: 30_000,
	}, async () => {
		const waiting = Promise.allSettled(
			Array.from({ length: CLIENT_TRANSPORT.maxPending }, (_, index) =>
				invokeClient("add_repo_path", { path: `/repo/${index}` }),
			),
		);
		const { socket } = await sent(CLIENT_TRANSPORT.maxPending);
		const next = invokeClient("write_terminal_surface", {
			owner: { kind: "workspace", workspacePath: "/repo" },
			attachmentId: "a",
			sequence: 0,
			data: "echo hi\n",
		});
		const observed = observeResult(next);
		const current = await sent(CLIENT_TRANSPORT.maxPending + 1);
		expect(socket.proposals).toHaveBeenCalledTimes(
			CLIENT_TRANSPORT.maxPending + 1,
		);
		socket.message({
			request_id: current.frames[CLIENT_TRANSPORT.maxPending].request_id,
			result: null,
		});
		await expect(next).resolves.toBeNull();
		expect(observed.rejected).not.toHaveBeenCalled();
		window.dispatchEvent(new Event("pagehide"));
		await waiting;
	});
	it.each(["get_current_branch", "detach_terminal_surface"] as const)(
		"%sのdisconnected通知は結果不明で要求を解放し接続を維持する",
		async (command) => {
			const result =
				command === "get_current_branch"
					? invokeClient(command, { repoPath: "/repo" })
					: invokeClient(command, { attachmentId: "a" });
			const rejected = expect(result).rejects.toMatchObject({
				state: "unknown",
			});
			const { socket, frames } = await sent();
			const requestId = frames[0].request_id;
			socket.control({ operationStatus: { requestId, state: "disconnected" } });
			await rejected;
			expect(getClientStatus().operations).toEqual([]);
			expect(getClientStatus().connected).toBe(true);
			expect(socket.close).not.toHaveBeenCalled();
			socket.sendFrame.mockClear();
			retryClientOperation(requestId);
			expect(socket.sendFrame).not.toHaveBeenCalled();
			const next = invokeClient("get_current_branch", { repoPath: "/next" });
			const current = await sent();
			socket.message({
				request_id: current.frames[0].request_id,
				result: "main",
			});
			await expect(next).resolves.toBe("main");
		},
	);
});

describe("Rustの復旧指示と監視所有の回帰", () => {
	beforeEach(() => {
		vi.useFakeTimers();
		vi.clearAllMocks();
		FakeWebSocket.instances = [];
		FakeWebSocket.nextDecision = null;
		FakeWebSocket.hello = {};
		FakeWebSocket.respond = null;
		FakeWebSocket.failOpen = false;
		FakeWebSocket.autoOpen = true;
		FakeWebSocket.heartbeatResponds = true;
		FakeWebSocket.instanceId = "backend-1";
		vi.stubGlobal("WebSocket", FakeWebSocket);
		vi.mocked(invoke).mockResolvedValue({
			url: "ws://127.0.0.1:123/v1/client",
			authSubprotocol: "client",
		});
	});
	afterEach(() => {
		window.dispatchEvent(new Event("pagehide"));
		vi.useRealTimers();
		vi.unstubAllGlobals();
	});

	it.each([false, true])(
		"期限後の監視成功は所有先へ対応付け、破棄済みなら回収を指示する: %s",
		async (disposed) => {
			const ready = vi.fn();
			const error = vi.fn();
			const listener = vi.fn();
			const off = await listenClient("file-change", listener);
			const stop = watchClient(
				"start_watching",
				{ path: "/repo" },
				ready,
				error,
			);
			const { socket, frames } = await sent();
			await vi.advanceTimersByTimeAsync(30_000);
			expect(error).not.toHaveBeenCalled();
			expect(getClientStatus().operations).toContainEqual(
				expect.objectContaining({ state: "unknown" }),
			);
			if (disposed) stop();
			socket.message({ request_id: frames[0].request_id, result: 42 });
			await vi.advanceTimersByTimeAsync(0);
			const ack = socket.sendFrame.mock.calls
				.map(([bytes]) => fromBinary(EnvelopeSchema, bytes).body)
				.find((body) => body.case === "requestAck");
			expect(ack).toMatchObject({
				case: "requestAck",
				value: { requestId: frames[0].request_id, releaseWatch: disposed },
			});
			if (disposed) expect(ready).not.toHaveBeenCalled();
			else {
				socket.message({ request_id: frames[0].request_id, result: 42 });
				expect(ready).toHaveBeenCalledTimes(1);
				expect(ready).toHaveBeenCalledWith(-1);
				socket.message({
					status: "push",
					event: "file-change",
					payload: { watcher_id: 42, path: "/repo/a", kind: "modify" },
				});
				expect(listener).toHaveBeenCalledWith({
					payload: expect.objectContaining({ watcher_id: -1 }),
				});
				stop();
				const stopping = await sent(2);
				expect(stopping.frames[1].args).toEqual({ watcherId: 42 });
				socket.message({
					request_id: stopping.frames[1].request_id,
					result: null,
				});
			}
			off();
		},
	);

	it("Rust発行の同一性と対象識別を次の要求へそのまま渡す", async () => {
		FakeWebSocket.nextDecision = {
			state: "ready",
			fingerprint: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
			orderingTarget: "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=",
		};
		const original = invokeClient("add_repo_path", { path: "/repo" });
		const observed = observeResult(original);
		const first = await sent();
		await vi.advanceTimersByTimeAsync(30_000);
		expect(observed.rejected).not.toHaveBeenCalled();
		expect(observed.resolved).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ state: "unknown" }),
		);
		FakeWebSocket.nextDecision = {
			state: "bound",
			operationId: first.frames[0].request_id,
		};
		const retry = invokeClient("add_repo_path", { path: "/repo" });
		expect(
			first.socket.proposals.mock.lastCall?.[0].request.predecessors[0],
		).toMatchObject({
			requestId: first.frames[0].request_id,
			fingerprint: new Uint8Array(32).fill(1),
			orderingTarget: new Uint8Array(32).fill(2),
		});
		first.socket.message({
			request_id: first.frames[0].request_id,
			result: true,
		});
		await expect(original).resolves.toBe(true);
		await expect(retry).resolves.toBe(true);
	});

	it("確定して受領した後続削除を先行追加の復旧照会に保持する", async () => {
		const identity = {
			state: "ready",
			fingerprint: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
			orderingTarget: "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=",
		};
		FakeWebSocket.nextDecision = identity;
		const original = observeResult(
			invokeClient("add_repo_path", { path: "/repo" }),
		);
		const first = await sent();
		FakeWebSocket.nextDecision = identity;
		const removal = invokeClient("remove_repo_path", { path: "/repo" });
		const both = await sent(2);
		both.socket.message({
			request_id: both.frames[1].request_id,
			result: true,
		});
		await removal;
		expect(
			both.socket.sendFrame.mock.calls.map(
				([bytes]) => fromBinary(EnvelopeSchema, bytes).body,
			),
		).toContainEqual(
			expect.objectContaining({
				case: "requestAck",
				value: expect.objectContaining({
					requestId: both.frames[1].request_id,
				}),
			}),
		);
		first.socket.onclose?.();
		FakeWebSocket.instanceId = "backend-2";
		await vi.advanceTimersByTimeAsync(1000);
		const socket = FakeWebSocket.instances[1];
		const query = socket.sendFrame.mock.calls
			.map(([bytes]) => fromBinary(EnvelopeSchema, bytes).body)
			.find((body) => body.case === "operationQuery");
		expect(query).toMatchObject({
			case: "operationQuery",
			value: {
				requestId: first.frames[0].request_id,
				request: {
					successors: [
						{
							requestId: both.frames[1].request_id,
							command: "remove_repo_path",
							orderingTarget: new Uint8Array(32).fill(2),
						},
					],
				},
			},
		});
		socket.control({
			operationStatus: {
				requestId: first.frames[0].request_id,
				state: "unknown",
			},
		});
		expect(
			socket.sendFrame.mock.calls.map(
				([bytes]) => fromBinary(EnvelopeSchema, bytes).body.case,
			),
		).not.toContain("request");
		expect(original.resolved).not.toHaveBeenCalled();
		expect(original.rejected).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ command: "add_repo_path", state: "unknown" }),
		);
	});

	it("世代変更時に新規監視と既存監視の復旧が交差しても通知と停止先が衝突しない", async () => {
		const first = vi.fn();
		const second = vi.fn();
		const errors = vi.fn();
		const stopFirst = watchClient(
			"start_watching",
			{ path: "/a" },
			first,
			errors,
		);
		const initial = await sent();
		initial.socket.message({
			request_id: initial.frames[0].request_id,
			result: 1,
		});
		initial.socket.onclose?.();
		const stopSecond = watchClient(
			"start_watching",
			{ path: "/b" },
			second,
			errors,
		);
		FakeWebSocket.instanceId = "backend-2";
		await vi.advanceTimersByTimeAsync(1_000);
		const recovered = await sent(2, 1);
		const newWatch = recovered.frames.find(
			(frame) => (frame.args as { path: string }).path === "/b",
		);
		const restoredWatch = recovered.frames.find(
			(frame) => (frame.args as { path: string }).path === "/a",
		);
		await vi.advanceTimersByTimeAsync(30_000);
		recovered.socket.message({ request_id: newWatch?.request_id, result: 1 });
		recovered.socket.message({
			request_id: restoredWatch?.request_id,
			result: 2,
		});
		expect(first).toHaveBeenLastCalledWith(-1);
		expect(second).toHaveBeenLastCalledWith(-2);
		const changed = vi.fn();
		const off = await listenClient("file-change", changed);
		for (const id of [1, 2])
			recovered.socket.message({
				status: "push",
				event: "file-change",
				payload: { watcher_id: id, path: "/changed", kind: "modify" },
			});
		expect(
			changed.mock.calls.map(([event]) => event.payload.watcher_id),
		).toEqual([-2, -1]);
		stopFirst();
		stopSecond();
		const stopped = await sent(4, 1);
		expect(stopped.frames.slice(2).map((frame) => frame.args)).toEqual([
			{ watcherId: 2 },
			{ watcherId: 1 },
		]);
		for (const frame of stopped.frames.slice(2))
			recovered.socket.message({ request_id: frame.request_id, result: null });
		off();
	});

	it("Helloの操作期限と生存確認間隔・応答期限・tickを適用する", async () => {
		FakeWebSocket.hello = {
			heartbeatIntervalMs: "200",
			heartbeatTimeoutMs: "80",
			connectTimeoutMs: "400",
			reconnectIntervalMs: "60",
			sleepGapMs: "10000",
			tickIntervalMs: "20",
			deadlinesMs: { get_current_branch: "60", create_worktree: "600" },
		};
		FakeWebSocket.heartbeatResponds = false;
		const read = invokeClient("get_current_branch", { repoPath: "/repo" });
		const expired = expect(read).rejects.toMatchObject({ state: "unknown" });
		const long = invokeClient("create_worktree", {
			repoPath: "/repo",
			branch: "new",
			createBranch: true,
			baseBranch: "main",
		});
		const observed = observeResult(long);
		await vi.advanceTimersByTimeAsync(0);
		const socket = FakeWebSocket.instances[0];
		await vi.advanceTimersByTimeAsync(59);
		expect(getClientStatus().operations).toEqual([]);
		await vi.advanceTimersByTimeAsync(1);
		await expired;
		expect(socket.close).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(219);
		expect(socket.close).not.toHaveBeenCalled();
		await vi.advanceTimersByTimeAsync(1);
		expect(socket.close).toHaveBeenCalledTimes(1);
		expect(getClientStatus().message).toContain("通信状態を確認できません");
		await vi.advanceTimersByTimeAsync(59);
		expect(FakeWebSocket.instances).toHaveLength(1);
		await vi.advanceTimersByTimeAsync(1);
		expect(FakeWebSocket.instances).toHaveLength(2);
		await vi.advanceTimersByTimeAsync(260);
		expect(observed.rejected).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({
				command: "create_worktree",
				state: "unknown",
				expired: true,
			}),
		);
	});

	it("復旧照会がpendingでも定期再照会で元の要求の確定結果を反映する", async () => {
		const result = invokeClient("add_repo_path", { path: "/repo" });
		const initial = await sent();
		initial.socket.onclose?.();
		await vi.advanceTimersByTimeAsync(1_000);
		const recovered = FakeWebSocket.instances[1];
		recovered.control({
			operationStatus: {
				requestId: initial.frames[0].request_id,
				state: "pending",
			},
		});
		recovered.sendFrame.mockImplementation((bytes) => {
			const body = fromBinary(EnvelopeSchema, bytes).body;
			if (body.case === "operationQuery")
				recovered.control({
					response: {
						requestId: body.value.requestId,
						result: { addRepoPath: { value: true } },
					},
				});
		});
		await vi.advanceTimersByTimeAsync(5_000);
		await expect(result).resolves.toBe(true);
		expect(getClientStatus().operations).toEqual([]);
		expect(
			FakeWebSocket.instances
				.flatMap((socket) => socket.sendFrame.mock.calls)
				.filter(
					([bytes]) =>
						fromBinary(EnvelopeSchema, bytes).body.case === "request",
				),
		).toHaveLength(1);
	});

	it("期限切れの冪等要求の利用者再試行をRustへ照会し同じIDで送り直す", async () => {
		const result = invokeClient("add_repo_path", { path: "/repo" });
		const observed = observeResult(result);
		const initial = await sent();
		await vi.advanceTimersByTimeAsync(30_000);
		expect(observed.rejected).not.toHaveBeenCalled();
		expect(observed.resolved).not.toHaveBeenCalled();
		expect(getClientStatus().operations).toContainEqual(
			expect.objectContaining({ state: "unknown" }),
		);
		initial.socket.sendFrame.mockImplementation((bytes) => {
			const body = fromBinary(EnvelopeSchema, bytes).body;
			if (body.case === "operationQuery") {
				expect(body.value.expired).toBe(false);
				initial.socket.control({
					operationStatus: { requestId: body.value.requestId, state: "ready" },
				});
			}
		});
		retryClientOperation(initial.frames[0].request_id);
		const replay = await sent(2);
		expect(replay.frames[1]).toEqual(initial.frames[0]);
		replay.socket.message({
			request_id: replay.frames[1].request_id,
			result: true,
		});
		await expect(result).resolves.toBe(true);
		expect(getClientStatus().operations).toEqual([]);
	});
});

describe("購読画面のWS回復", () => {
	beforeEach(() => {
		vi.useFakeTimers();
		vi.clearAllMocks();
		FakeWebSocket.instances = [];
		FakeWebSocket.nextDecision = null;
		FakeWebSocket.hello = {};
		FakeWebSocket.failOpen = false;
		FakeWebSocket.autoOpen = true;
		FakeWebSocket.heartbeatResponds = true;
		FakeWebSocket.instanceId = "backend-1";
		vi.stubGlobal("WebSocket", FakeWebSocket);
		vi.mocked(invoke).mockResolvedValue({
			url: "ws://127.0.0.1:123/v1/client",
			authSubprotocol: "client",
		});
	});
	afterEach(() => {
		window.dispatchEvent(new Event("pagehide"));
		FakeWebSocket.respond = null;
		vi.useRealTimers();
		vi.unstubAllGlobals();
	});

	it("初回接続前に取得が期限切れになった画面も接続回復で現在状態を取得する", async () => {
		FakeWebSocket.failOpen = true;
		FakeWebSocket.respond = () => ["/recovered"];
		const view = renderHook(() => useRepoList());
		await act(() => vi.advanceTimersByTimeAsync(10_000));
		expect(view.result.current.repoPaths).toEqual([]);
		FakeWebSocket.failOpen = false;
		await act(() => vi.advanceTimersByTimeAsync(1_000));
		expect(view.result.current.repoPaths).toEqual(["/recovered"]);
		view.unmount();
	});

	it("初回の一覧取得を重複させず、pushのない再接続で全購読先の現在状態を取得する", async () => {
		let changed = false;
		const branch = {
			name: "new",
			worktree_path: "/repo/new",
			is_main_worktree: false,
			is_merged: false,
			has_upstream: false,
			ahead: 0,
			behind: 0,
			base_ahead: 0,
			dirty_count: 0,
		};
		FakeWebSocket.respond = (command) => {
			switch (command) {
				case "getRepoPaths":
					return changed ? ["/repo/new"] : ["/repo"];
				case "listBranchesWithStatusSnapshot":
					return {
						version: changed ? 2 : 1,
						stale: false,
						loading: false,
						limited: false,
						branches: changed ? [branch] : [],
						worktree_display_groups: { working_areas: changed ? [branch] : [] },
					};
				case "getCachedPrStatus":
					return { open_prs: {}, merged_branches: [] };
				case "startWatching":
				case "startGitDirWatching":
					return 42;
				case "getAutomationConfigDir":
					return "/config";
				case "listWorkflows":
					return changed
						? [
								{
									name: "new",
									description: "new",
									builtin: false,
									is_running: false,
									sourceFormat: "yaml",
								},
							]
						: [];
				case "diagnoseAllCmd":
					return {
						items: [],
						workflow_summaries: {},
						facet_summaries: {},
						facet_usage: {},
					};
				default:
					return null;
			}
		};
		const gitRefresh = vi.fn();
		const view = renderHook(() => {
			const repos = useRepoList();
			const worktrees = useWorktreeList("/repo");
			const automation = useAutomation(true);
			useGitEventRefresh("/repo", gitRefresh);
			return { repos, worktrees, automation };
		});
		const agentRefresh = vi.fn();
		const offAgent = subscribeAgentSessionChanged(agentRefresh);
		await act(() => vi.advanceTimersByTimeAsync(0));
		expect(view.result.current.repos.repoPaths).toEqual(["/repo"]);
		const initial = FakeWebSocket.instances[0];
		const calls = initial.sendFrame.mock.calls.map(
			([bytes]) => fromBinary(EnvelopeSchema, bytes).body,
		);
		for (const command of [
			"getRepoPaths",
			"listBranchesWithStatusSnapshot",
			"listWorkflows",
		])
			expect(
				calls.filter(
					(body) =>
						body.case === "request" && body.value.command.case === command,
				),
			).toHaveLength(1);
		expect(gitRefresh).not.toHaveBeenCalled();
		expect(agentRefresh).not.toHaveBeenCalled();
		await act(async () => {
			initial.onclose?.();
			changed = true;
			await vi.advanceTimersByTimeAsync(1_300);
		});
		expect(view.result.current.repos.repoPaths).toEqual(["/repo/new"]);
		expect(
			view.result.current.worktrees.branches.map((item) => item.name),
		).toEqual(["new"]);
		expect(
			view.result.current.automation.workflows.map((item) => item.name),
		).toEqual(["new"]);
		expect(gitRefresh).toHaveBeenCalledTimes(1);
		expect(agentRefresh).toHaveBeenCalledWith({});
		expect(
			vi
				.mocked(invoke)
				.mock.calls.every(([command]) => command === "get_client_endpoint"),
		).toBe(true);
		view.unmount();
		offAgent();
	});
	it.each(["reconnect", "late"])(
		"実clientSocketの%sから設定画面を再取得する",
		async (trigger) => {
			let changed = false;
			const respond = (command: string) => {
				switch (command) {
					case "getExternalEditor":
						return changed ? "zed" : "code";
					case "detectEditors":
						return [
							{ name: "Code", path: "code" },
							{ name: "Zed", path: "zed" },
						];
					case "getWorkflowConfig":
						return { approval_auto_approve: changed };
					case "getProviderAvailability":
						return { providers: [] };
					case "getAppSettings":
						return {
							close_to_tray: true,
							auto_launch: false,
							start_minimized: false,
						};
					case "listWorkflows":
						return [];
					case "diagnoseAllCmd":
						return {
							items: [],
							workflow_summaries: {},
							facet_summaries: {},
							facet_usage: {},
						};
					case "getAutomationConfigDir":
						return "/config";
					case "startWatching":
						return 42;
					default:
						return null;
				}
			};
			FakeWebSocket.respond = respond;
			const view = render(
				createElement(SettingsModal, {
					open: true,
					onOpenChange: vi.fn(),
					settings: DEFAULT_SETTINGS,
					onSave: vi.fn(),
				}),
			);
			await act(() => vi.advanceTimersByTimeAsync(0));
			fireEvent.click(screen.getByText("Editor"));
			expect(
				screen.getByRole("combobox", { name: "External Editor" }),
			).toHaveTextContent("Code");
			const socket = FakeWebSocket.instances[0];
			if (trigger === "reconnect") {
				await act(async () => {
					changed = true;
					socket.onclose?.();
					await vi.advanceTimersByTimeAsync(1_000);
				});
			} else {
				FakeWebSocket.respond = null;
				const operation = invokeClient("update_external_editor", {
					editor: "zed",
				});
				const requests = socket.sendFrame.mock.calls.map(
					([bytes]) => fromBinary(EnvelopeSchema, bytes).body,
				);
				const request = [...requests]
					.reverse()
					.find(
						(body) =>
							body.case === "request" &&
							body.value.command.case === "updateExternalEditor",
					);
				if (request?.case !== "request") throw new Error("missing update");
				await act(() => vi.advanceTimersByTimeAsync(30_000));
				expect(getClientStatus().operations).toContainEqual(
					expect.objectContaining({
						command: "update_external_editor",
						state: "unknown",
					}),
				);
				await act(async () => {
					changed = true;
					FakeWebSocket.respond = respond;
					socket.message({ request_id: request.value.requestId, result: null });
					await operation;
				});
			}
			expect(
				screen.getByRole("combobox", { name: "External Editor" }),
			).toHaveTextContent("Zed");
			fireEvent.click(screen.getByText("Agent"));
			expect(
				screen.getByRole("checkbox", { name: "Approval auto-approve" }),
			).toBeChecked();
			const requests = FakeWebSocket.instances
				.flatMap((socket) => socket.sendFrame.mock.calls)
				.map(([bytes]) => fromBinary(EnvelopeSchema, bytes).body);
			for (const command of [
				"getAppSettings",
				"getExternalEditor",
				"detectEditors",
				"getWorkflowConfig",
			]) {
				expect(
					requests.filter(
						(body) =>
							body.case === "request" && body.value.command.case === command,
					),
				).toHaveLength(2);
			}
			expect(
				vi
					.mocked(invoke)
					.mock.calls.every(([command]) => command === "get_client_endpoint"),
			).toBe(true);
			view.unmount();
		},
	);

	it.each(["deadline", "disconnect"])(
		"背景設定保存の%sで操作を無効化せず元の要求を照会して保存を完了する",
		async (trigger) => {
			FakeWebSocket.respond = (command) => {
				switch (command) {
					case "getAppSettings":
						return {
							close_to_tray: true,
							auto_launch: false,
							start_minimized: false,
						};
					case "getWorkflowConfig":
						return { approval_auto_approve: false };
					case "getProviderAvailability":
						return { providers: [] };
					case "detectEditors":
					case "listWorkflows":
						return [];
					case "getExternalEditor":
					case "getAutomationConfigDir":
						return "";
					default:
						return null;
				}
			};
			const onSave = vi.fn();
			const view = render(
				createElement(SettingsModal, {
					open: true,
					onOpenChange: vi.fn(),
					settings: DEFAULT_SETTINGS,
					onSave,
				}),
			);
			await act(() => vi.advanceTimersByTimeAsync(0));
			FakeWebSocket.respond = null;
			fireEvent.click(screen.getByText("Background"));
			fireEvent.click(
				screen.getByRole("checkbox", { name: "Minimize to tray on close" }),
			);
			const save = screen.getByRole("button", { name: "Save" });
			await act(async () => fireEvent.click(save));
			expect(save).toBeDisabled();
			const updates = () =>
				FakeWebSocket.instances
					.flatMap((socket) => socket.sendFrame.mock.calls)
					.map(([bytes]) => fromBinary(EnvelopeSchema, bytes).body)
					.filter(
						(body) =>
							body.case === "request" &&
							body.value.command.case === "updateAppSettings",
					);
			expect(updates()).toHaveLength(1);
			const original = updates()[0];
			if (original.case !== "request") throw new Error("Missing update");
			const requestId = original.value.requestId;
			await act(async () => {
				if (trigger === "deadline")
					await vi.advanceTimersByTimeAsync(
						CLIENT_TRANSPORT.commands.update_app_settings.deadlineMs,
					);
				else {
					FakeWebSocket.instances[0].onclose?.();
					await vi.advanceTimersByTimeAsync(1_000);
				}
			});
			fireEvent.click(screen.getByText("Appearance"));
			expect(screen.getByRole("alert")).toHaveTextContent(
				"操作結果を確認できません",
			);
			expect(save).toBeEnabled();
			expect(save).toHaveTextContent("元の操作の結果を確認");
			expect(save.querySelector(".animate-spin")).toBeNull();
			fireEvent.click(save);
			const socket =
				FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
			if (!socket) throw new Error("Missing connection");
			const queries = socket.sendFrame.mock.calls
				.map(([bytes]) => fromBinary(EnvelopeSchema, bytes).body)
				.filter((body) => body.case === "operationQuery");
			expect(queries[queries.length - 1]?.value).toMatchObject({
				requestId,
				request: { userRetry: true },
			});
			expect(updates()).toHaveLength(1);
			expect(onSave).toHaveBeenCalledTimes(1);
			await act(async () => {
				socket.control({
					response: { requestId, result: { updateAppSettings: {} } },
				});
			});
			expect(screen.queryByRole("alert")).not.toBeInTheDocument();
			expect(save).toHaveTextContent("Save");
			expect(save).toBeDisabled();
			fireEvent.click(screen.getByText("Background"));
			view.unmount();
		},
	);

	it("期限後の未受理caller要求は自動送信せず利用者再試行で同じ要求を一度送る", async () => {
		const result = invokeClient("request_application_quit", {
			request: {
				request_id: "quit-original",
				intent: { type: "exit", code: 0 },
			},
		});
		const observed = observeResult(result);
		const initial = await sent();
		initial.socket.onclose?.();
		FakeWebSocket.failOpen = true;
		await vi.advanceTimersByTimeAsync(120_000);
		FakeWebSocket.failOpen = false;
		FakeWebSocket.instanceId = "backend-2";
		await vi.advanceTimersByTimeAsync(1_000);
		const recovered =
			FakeWebSocket.instances[FakeWebSocket.instances.length - 1];
		const query = fromBinary(
			EnvelopeSchema,
			recovered.sendFrame.mock.calls[0][0],
		).body;
		expect(query).toMatchObject({
			case: "operationQuery",
			value: { expired: true, request: { userRetry: false } },
		});
		recovered.control({
			operationStatus: {
				requestId: initial.frames[0].request_id,
				state: "unknown",
			},
		});
		await vi.advanceTimersByTimeAsync(5_000);
		expect(
			recovered.sendFrame.mock.calls.every(
				([bytes]) =>
					fromBinary(EnvelopeSchema, bytes).body.case === "operationQuery",
			),
		).toBe(true);
		retryClientOperation(initial.frames[0].request_id);
		expect(
			fromBinary(
				EnvelopeSchema,
				recovered.sendFrame.mock.calls[
					recovered.sendFrame.mock.calls.length - 1
				][0],
			).body,
		).toMatchObject({
			case: "operationQuery",
			value: { expired: false, request: { userRetry: true } },
		});
		recovered.control({
			operationStatus: {
				requestId: initial.frames[0].request_id,
				state: "ready",
			},
		});
		const replay = await sent(1, FakeWebSocket.instances.length - 1);
		expect(replay.frames[0]).toEqual(initial.frames[0]);
		expect(observed.rejected).not.toHaveBeenCalled();
		recovered.message({
			request_id: replay.frames[0].request_id,
			error: { code: "CONFIRMED" },
		});
		await expect(result).rejects.toMatchObject({ code: "CONFIRMED" });
	});
	it.each(["get_current_branch", "add_repo_path"] as const)(
		"未送信%sの照会応答が交差しても一度だけ送る",
		async (command) => {
			const initial = invokeClient("get_current_branch", {
				repoPath: "/initial",
			});
			const { socket, frames } = await sent();
			socket.message({ request_id: frames[0].request_id, result: "main" });
			await initial;
			const send = socket.send;
			const queries: Array<{ requestId: string; queryId: string }> = [];
			socket.send = (bytes) => {
				const body = fromBinary(EnvelopeSchema, bytes).body;
				if (body.case === "operationQuery") {
					queries.push({
						requestId: body.value.requestId,
						queryId: body.value.queryId,
					});
					return;
				}
				send(bytes);
			};
			const result = invokeClient(
				command,
				command === "get_current_branch" ? { repoPath: "/a" } : { path: "/a" },
			);
			observeResult(result);
			const other = invokeClient("get_current_branch", { repoPath: "/b" });
			observeResult(other);
			const target = queries[0];
			socket.control({ operationStatus: { ...queries[1], state: "ready" } });
			const withOther = await sent(2);
			socket.message({
				request_id: withOther.frames[1].request_id,
				result: "other",
			});
			const repeated = queries.filter(
				(query) => query.requestId === target.requestId,
			);
			expect(repeated).toHaveLength(2);
			for (const query of [repeated[1], repeated[0], repeated[1]])
				socket.control({ operationStatus: { ...query, state: "ready" } });
			const current = await sent(3);
			expect(
				current.frames.filter((frame) => frame.request_id === target.requestId),
			).toHaveLength(1);
			socket.control({
				operationStatus: { requestId: target.requestId, state: "pending" },
			});
			expect(getClientStatus().operations).toEqual([]);
			socket.message({
				request_id: target.requestId,
				result: command === "get_current_branch" ? "main" : true,
			});
			await result;
		},
	);

	it("backendで未送信期限切れが確定した要求を未実行の表示に保持する", async () => {
		FakeWebSocket.nextDecision = { state: "not_sent" };
		await expect(
			invokeClient("add_repo_path", { path: "/never-sent" }),
		).rejects.toMatchObject({ state: "not_sent" });
		expect(getClientStatus().operations).toEqual([
			expect.objectContaining({
				command: "add_repo_path",
				state: "not_sent",
				expired: true,
			}),
		]);
		expect(FakeWebSocket.instances[0].sendFrame).not.toHaveBeenCalled();
	});

	it.each(["start_watching", "start_git_dir_watching"] as const)(
		"未受領%sが期限後に解放されても必要な監視を別の開始として復旧する",
		async (command) => {
			const ready = vi.fn();
			const stop = watchClient(
				command,
				command === "start_watching"
					? { path: "/repo" }
					: { repoPath: "/repo" },
				ready,
			);
			const initial = await sent();
			initial.socket.onclose?.();
			await vi.advanceTimersByTimeAsync(30_000);
			const socket = FakeWebSocket.instances[1];
			const originalId = initial.frames[0].request_id;
			socket.control({
				operationStatus: { requestId: originalId, state: "watch_released" },
			});
			const restored = await sent(1, 1);
			expect(restored.frames[0].request_id).not.toBe(originalId);
			expect(getClientStatus().operations).toContainEqual(
				expect.objectContaining({
					id: originalId,
					state: "unknown",
					expired: true,
				}),
			);
			socket.message({ request_id: restored.frames[0].request_id, result: 43 });
			expect(ready).toHaveBeenCalledExactlyOnceWith(-1);
			const changed = vi.fn();
			const off = await listenClient("file-change", changed);
			socket.message({
				status: "push",
				event: "file-change",
				payload: { watcher_id: 43, path: "/repo/a", kind: "modify" },
			});
			expect(changed).toHaveBeenCalledWith({
				payload: expect.objectContaining({ watcher_id: -1 }),
			});
			off();
			stop();
		},
	);

	it("保持期間後の監視成功は受領確認が拒否されたら有効にせず実在する監視を取得する", async () => {
		const refresh = vi.fn();
		const off = await listenClient("file-change", vi.fn(), refresh);
		const ready = vi.fn();
		const stop = watchClient("start_watching", { path: "/repo" }, ready);
		const initial = await sent();
		await vi.advanceTimersByTimeAsync(300_000);
		const send = initial.socket.send;
		initial.socket.send = (bytes) => {
			const body = fromBinary(EnvelopeSchema, bytes).body;
			if (
				body.case === "requestAck" &&
				body.value.requestId === initial.frames[0].request_id &&
				body.value.confirmWatch
			) {
				initial.socket.control({
					operationStatus: {
						requestId: body.value.requestId,
						state: "watch_released",
					},
				});
				return;
			}
			send(bytes);
		};
		initial.socket.message({
			request_id: initial.frames[0].request_id,
			result: 42,
		});
		expect(ready).not.toHaveBeenCalled();
		const restored = await sent(2);
		initial.socket.message({
			request_id: restored.frames[1].request_id,
			result: 43,
		});
		expect(refresh).toHaveBeenCalledTimes(1);
		expect(ready).toHaveBeenCalledExactlyOnceWith(-1);
		initial.socket.message({
			request_id: initial.frames[0].request_id,
			result: 42,
		});
		expect(ready).toHaveBeenCalledTimes(1);
		stop();
		const stopped = await sent(3);
		expect(stopped.frames[2].args).toEqual({ watcherId: 43 });
		off();
	});

	it.each(["create_agent_session", "start_workflow"] as const)(
		"%sの120秒期限で結果不明を通知し、再送せず元の遅延結果を届ける",
		async (command) => {
			const onUncertain = vi.fn();
			const onResult = vi.fn();
			const operation = invokeClient(
				command,
				command === "create_agent_session"
					? {
							workspaceIdentity: "/repo",
							worktreePath: "/repo",
							provider: "codex",
							rows: 24,
							cols: 80,
							callerRequestId: "create.original",
						}
					: {
							workflowName: "release",
							worktreePath: "/repo",
							request: "release request",
						},
				{ onUncertain },
			).then(onResult);
			const initial = await sent();
			await vi.advanceTimersByTimeAsync(119_000);
			expect(onUncertain).not.toHaveBeenCalled();
			await vi.advanceTimersByTimeAsync(1000);
			expect(onUncertain).toHaveBeenCalledExactlyOnceWith(
				expect.objectContaining({
					requestId: initial.frames[0].request_id,
					state: "unknown",
				}),
			);
			expect(onResult).not.toHaveBeenCalled();
			expect(getClientStatus()).toMatchObject({
				connected: true,
				operations: [
					{
						id: initial.frames[0].request_id,
						command,
						state: "unknown",
						expired: true,
					},
				],
			});
			expect((await sent()).frames).toHaveLength(1);
			initial.socket.message({
				request_id: initial.frames[0].request_id,
				result: "created-id",
			});
			await operation;
			expect(onResult).toHaveBeenCalledExactlyOnceWith("created-id");
			expect(getClientStatus().operations).toEqual([]);
		},
	);

	it.each(["deadline", "disconnect", "unknown"])(
		"作成要求の%sを通知した後も元の成功結果を届ける",
		async (cause) => {
			const onUncertain = vi.fn();
			const operation = invokeClient(
				"create_worktree",
				{
					repoPath: "/repo",
					branch: "new",
					createBranch: true,
					baseBranch: "main",
				},
				{ onUncertain },
			);
			const initial = await sent();
			if (cause === "deadline") await vi.advanceTimersByTimeAsync(120_000);
			else if (cause === "disconnect") initial.socket.onclose?.();
			else
				initial.socket.control({
					operationStatus: {
						requestId: initial.frames[0].request_id,
						state: "unknown",
					},
				});
			expect(onUncertain).toHaveBeenCalledWith(
				expect.objectContaining({ state: "unknown" }),
			);
			const entry = {
				name: "new",
				path: "/repo/new",
				branch: "new",
				is_main: false,
				is_locked: false,
				dirty_count: 0,
				base_branch: "main",
			};
			if (cause === "disconnect") {
				await vi.advanceTimersByTimeAsync(1000);
				FakeWebSocket.instances[1].control({
					response: {
						requestId: initial.frames[0].request_id,
						result: { createWorktree: entry },
					},
				});
			} else
				initial.socket.message({
					request_id: initial.frames[0].request_id,
					result: entry,
				});
			await expect(operation).resolves.toEqual(entry);
		},
	);

	it("異なる対象範囲の確定応答で同じ範囲の後続操作の保護情報を失わない", async () => {
		const crashIdentity = {
			state: "ready",
			fingerprint: "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=",
			orderingTarget: "AgICAgICAgICAgICAgICAgICAgICAgICAgICAgICAgI=",
		};
		FakeWebSocket.nextDecision = crashIdentity;
		observeResult(invokeClient("update_crash_reporting", { enabled: true }));
		const original = await sent();
		FakeWebSocket.nextDecision = crashIdentity;
		const later = invokeClient("update_crash_reporting", { enabled: false });
		const pair = await sent(2);
		pair.socket.message({
			request_id: pair.frames[1].request_id,
			result: null,
		});
		await later;
		FakeWebSocket.nextDecision = {
			...crashIdentity,
			orderingTarget: "AwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwMDAwM=",
		};
		const unrelated = invokeClient("report_mounted_xterm_count", { count: 3 });
		const all = await sent(3);
		all.socket.message({ request_id: all.frames[2].request_id, result: null });
		await unrelated;
		original.socket.onclose?.();
		FakeWebSocket.instanceId = "backend-2";
		await vi.advanceTimersByTimeAsync(1000);
		const queries = FakeWebSocket.instances[1].sendFrame.mock.calls.map(
			([bytes]) => fromBinary(EnvelopeSchema, bytes).body,
		);
		expect(queries).toContainEqual(
			expect.objectContaining({
				case: "operationQuery",
				value: expect.objectContaining({
					request: expect.objectContaining({
						successors: [
							expect.objectContaining({
								requestId: pair.frames[1].request_id,
								command: "update_crash_reporting",
							}),
						],
					}),
				}),
			}),
		);
	});
	it("元の結果不明な作成に束縛した画面にも状態と確定結果を届ける", async () => {
		const args = {
			repoPath: "/repo",
			branch: "new",
			createBranch: true,
			baseBranch: "main",
		};
		const original = invokeClient("create_worktree", args);
		const initial = await sent();
		await vi.advanceTimersByTimeAsync(120_000);
		FakeWebSocket.nextDecision = {
			state: "bound",
			operationId: initial.frames[0].request_id,
		};
		const onUncertain = vi.fn();
		const retried = invokeClient("create_worktree", args, { onUncertain });
		expect(onUncertain).toHaveBeenCalledWith(
			expect.objectContaining({
				requestId: initial.frames[0].request_id,
				state: "unknown",
			}),
		);
		const entry = {
			name: "new",
			path: "/repo/new",
			branch: "new",
			is_main: false,
			is_locked: false,
			dirty_count: 0,
			base_branch: "main",
		};
		initial.socket.message({
			request_id: initial.frames[0].request_id,
			result: entry,
		});
		await expect(original).resolves.toEqual(entry);
		await expect(retried).resolves.toEqual(entry);
		await sent(1);
	});
	it("自動ackのboundは元の期限を更新せず利用者再試行だけが未受理要求を送る", async () => {
		const args = { callerRequestId: "attempt-original" };
		const original = invokeClient("acknowledge_application_attempt", args);
		const initial = await sent();
		await vi.advanceTimersByTimeAsync(
			CLIENT_TRANSPORT.commands.acknowledge_application_attempt.deadlineMs,
		);
		const originalId = initial.frames[0].request_id;
		const queries = () =>
			initial.socket.sendFrame.mock.calls
				.map(([bytes]) => fromBinary(EnvelopeSchema, bytes).body)
				.filter((body) => body.case === "operationQuery")
				.map((body) => body.value);
		const deadline = queries()[queries().length - 1]?.request?.deadlineUnixMs;
		const automatic = [];
		for (let i = 0; i < 3; i++) {
			FakeWebSocket.nextDecision = { state: "bound", operationId: originalId };
			automatic.push(invokeClient("acknowledge_application_attempt", args));
			await vi.advanceTimersByTimeAsync(5_000);
			expect(queries()[queries().length - 1]).toMatchObject({
				requestId: originalId,
				expired: true,
				request: { userRetry: false, deadlineUnixMs: deadline },
			});
			expect(getClientStatus().operations).toEqual([
				expect.objectContaining({
					id: originalId,
					state: "unknown",
					expired: true,
				}),
			]);
		}
		expect((await sent()).frames).toHaveLength(1);
		retryClientOperation(originalId);
		expect(queries()[queries().length - 1]).toMatchObject({
			requestId: originalId,
			expired: false,
			request: { userRetry: true },
		});
		initial.socket.control({
			operationStatus: { requestId: originalId, state: "ready" },
		});
		const replay = await sent(2);
		expect(replay.frames[1]).toEqual(initial.frames[0]);
		initial.socket.message({ request_id: originalId, result: null });
		await Promise.all([original, ...automatic]);
		expect(getClientStatus().operations).toEqual([]);
	});
	it.each([
		[
			"remove_worktree",
			{ repoPath: "/repo", worktreePath: "/repo/wt", force: false },
		],
		[
			"delete_branch",
			{ repoPath: "/repo", branchName: "feature", force: false },
		],
		["approve_workspace_node", { worktreePath: "/repo", nodeId: "approval" }],
		["update_workflow_config", { workflow: { approval_auto_approve: true } }],
		[
			"update_app_settings",
			{
				app: {
					close_to_tray: false,
					auto_launch: true,
					start_minimized: false,
				},
			},
		],
	] as const)(
		"%sの期限超過と切断を通知し、確定結果まで元の操作を保持する",
		async (command, args) => {
			const onUncertain = vi.fn();
			const result = invokeClient(command, args, { onUncertain });
			const observed = observeResult(result);
			const initial = await sent();
			await vi.advanceTimersByTimeAsync(
				CLIENT_TRANSPORT.commands[command].deadlineMs,
			);
			expect(onUncertain).toHaveBeenCalledWith(
				expect.objectContaining({ state: "unknown" }),
			);
			expect(observed.resolved).not.toHaveBeenCalled();
			expect(observed.rejected).not.toHaveBeenCalled();
			expect(getClientStatus().connected).toBe(true);
			onUncertain.mockClear();
			initial.socket.onclose?.();
			expect(onUncertain).toHaveBeenCalledWith(
				expect.objectContaining({ state: "unknown" }),
			);
			await vi.advanceTimersByTimeAsync(1000);
			FakeWebSocket.instances[1].control({
				response: {
					requestId: initial.frames[0].request_id,
					result: {
						[command.replace(/_([a-z])/g, (_, letter: string) =>
							letter.toUpperCase(),
						)]: {},
					},
				},
			});
			await result;
			expect(observed.resolved).toHaveBeenCalledTimes(1);
			expect(observed.rejected).not.toHaveBeenCalled();
			expect(getClientStatus().operations).toEqual([]);
		},
	);

	it.each([false, true])(
		"daemon の起動設定と更新要求の応答を同じ接続から shell へ適用する: crashReporting=%s",
		async (enabled) => {
			const initial = {
				closeToTray: false,
				startMinimized: true,
				crashReporting: !enabled,
				performanceTelemetry: true,
			};
			FakeWebSocket.hello = { desktopSettings: initial };
			const update = invokeClient("update_crash_reporting", { enabled });
			const observed = observeResult(update);
			const { socket, frames } = await sent();
			expect(frames[0]).toMatchObject({
				command: "update_crash_reporting",
				args: { enabled },
			});
			expect(invoke).toHaveBeenCalledWith("apply_desktop_settings", {
				settings: expect.objectContaining(initial),
			});
			const changed = { ...initial, crashReporting: enabled };
			expect(invoke).not.toHaveBeenCalledWith("apply_desktop_settings", {
				settings: expect.objectContaining(changed),
			});
			expect(observed.resolved).not.toHaveBeenCalled();
			expect(observed.rejected).not.toHaveBeenCalled();
			socket.control({
				response: {
					requestId: frames[0].request_id,
					desktopSettings: changed,
					result: { updateCrashReporting: {} },
				},
			});
			await expect(update).resolves.toBeNull();
			expect(invoke).toHaveBeenLastCalledWith("apply_desktop_settings", {
				settings: expect.objectContaining(changed),
			});
			expect(FakeWebSocket.instances).toHaveLength(1);
		},
	);
});
