import { invoke } from "@tauri-apps/api/core";
import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkflowState } from "@/hooks/useWorkflowState";
import type { WorkflowExecution } from "@/types/workflow";
import { invokeClient, listenClient } from "./clientSocket";

class FakeWebSocket {
	static instances: FakeWebSocket[] = [];
	static failOpen = false;
	static autoOpen = true;
	onopen: (() => void) | null = null;
	onerror: (() => void) | null = null;
	onclose: (() => void) | null = null;
	onmessage: ((event: { data: string }) => void) | null = null;
	send = vi.fn();
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
		this.onmessage?.({ data: JSON.stringify(frame) });
	}
}

async function sent(count = 1) {
	await vi.waitFor(() =>
		expect(FakeWebSocket.instances[0]?.send).toHaveBeenCalledTimes(count),
	);
	const socket = FakeWebSocket.instances[0];
	return {
		socket,
		frames: socket.send.mock.calls.map(([frame]) => JSON.parse(frame)),
	};
}

describe("clientSocket", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		FakeWebSocket.instances = [];
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
	it("backendのerrorと切断を失敗として返す", async () => {
		const failed = invokeClient("get_current_branch", {});
		const failedAssertion = expect(failed).rejects.toEqual({
			code: "INVALID_REQUEST",
		});
		const { socket, frames } = await sent();
		socket.message({
			request_id: frames[0].request_id,
			error: { code: "INVALID_REQUEST" },
		});
		await failedAssertion;
		const closed = invokeClient("get_current_branch", {});
		const closedAssertion = expect(closed).rejects.toThrow("connection closed");
		await sent(2);
		socket.onclose?.();
		await closedAssertion;
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
			workflowExecution: { id: "execution-1" },
		};
		socket.message({
			status: "push",
			event: "workflow-execution-changed",
			payload,
		});
		expect(listener).toHaveBeenCalledWith({ payload });
		socket.message({ status: "push", event: "file-change", payload: {} });
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
		await expect(invokeClient("get_current_branch", {})).rejects.toThrow(
			"unavailable",
		);
		const result = invokeClient("get_current_branch", {});
		const { socket, frames } = await sent();
		socket.message({ request_id: frames[0].request_id, result: "retry" });
		await expect(result).resolves.toBe("retry");
		expect(invoke).toHaveBeenCalledTimes(2);
	});
	it("応答のない要求はtimeoutで解放する", async () => {
		vi.useFakeTimers();
		const result = invokeClient("get_current_branch", {});
		const assertion = expect(result).rejects.toThrow("timed out");
		await vi.advanceTimersByTimeAsync(10_000);
		await assertion;
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
			const payload = { worktreePath: "/repo" };
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
			payload: { worktreePath: "/repo" },
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
		let current: Partial<WorkflowExecution> = {
			id: "execution-1",
			currentNode: "before",
		};
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
		expect(result.current.workflowExecution?.currentNode).toBe("before");
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
			payload: { worktreePath: "/repo" },
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
});
