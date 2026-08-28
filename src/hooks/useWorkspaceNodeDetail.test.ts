import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceNodeDetail } from "@/types/workspace-tree";
import {
	approveWorkspaceNode,
	retryWorkspaceNode,
	useWorkspaceNodeDetail,
} from "./useWorkspaceNodeDetail";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

type Listener = (event: { payload: never }) => void;

function detail(id: string, title = id): WorkspaceNodeDetail {
	return {
		id,
		title,
		status: "running",
		statusClassification: "active",
		submitReceived: false,
		stopReceived: false,
		hasArtifact: false,
		capabilities: { canApprove: false, canRetry: false, canClose: false },
		updatedAt: 1,
		content: { kind: "session", sessionId: `session-${id}` },
	};
}

function detailWithSession(
	id: string,
	title: string,
	sessionId: string,
): WorkspaceNodeDetail {
	return {
		...detail(id, title),
		content: { kind: "session", sessionId },
	};
}

function deferred<T>() {
	let resolve!: (value: T) => void;
	let reject!: (reason?: unknown) => void;
	const promise = new Promise<T>((res, rej) => {
		resolve = res;
		reject = rej;
	});
	return { promise, resolve, reject };
}

describe("useWorkspaceNodeDetail", () => {
	let listeners: Record<string, Listener[]>;
	let responses: Array<
		WorkspaceNodeDetail | null | Promise<WorkspaceNodeDetail | null>
	>;

	beforeEach(() => {
		vi.clearAllMocks();
		listeners = {};
		responses = [];
		mockListen.mockImplementation((event: string, listener: Listener) => {
			listeners[event] = [...(listeners[event] ?? []), listener];
			return Promise.resolve(vi.fn());
		});
		mockInvoke.mockImplementation((command: string) => {
			if (command === "get_workspace_node_detail") {
				return Promise.resolve(responses.shift() ?? null);
			}
			return Promise.resolve(null);
		});
	});

	it("loads detail by opaque node id", async () => {
		responses.push(detail("opaque-node"));
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "opaque-node" }),
		);

		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.detail).toEqual(detail("opaque-node"));
		expect(mockInvoke).toHaveBeenCalledWith("get_workspace_node_detail", {
			worktreePath: "/repo",
			nodeId: "opaque-node",
		});
	});

	it("loads the latest detail after subscriptions are established", async () => {
		const workflowListenerReady = deferred<() => void>();
		let currentDetail = detailWithSession(
			"node",
			"before session attach",
			"session-before",
		);

		mockListen.mockImplementation((event: string, listener: Listener) => {
			listeners[event] = [...(listeners[event] ?? []), listener];
			return event === "workflow-execution-changed"
				? workflowListenerReady.promise
				: Promise.resolve(vi.fn());
		});
		mockInvoke.mockImplementation((command: string) => {
			if (command === "get_workspace_node_detail") {
				return Promise.resolve(currentDetail);
			}
			return Promise.resolve(null);
		});

		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "node" }),
		);

		await waitFor(() =>
			expect(listeners["workflow-execution-changed"]?.length).toBe(1),
		);
		expect(mockInvoke).not.toHaveBeenCalled();

		currentDetail = detailWithSession(
			"node",
			"after session attach",
			"session-after",
		);

		await act(async () => {
			workflowListenerReady.resolve(vi.fn());
			await workflowListenerReady.promise;
		});
		await waitFor(() => expect(result.current.detail).toEqual(currentDetail));
		expect(mockInvoke).toHaveBeenCalledTimes(1);
	});

	it("reloads for a matching Worktree workflow event", async () => {
		responses.push(detail("node", "first"), detail("node", "workflow refresh"));
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "node" }),
		);
		await waitFor(() => expect(result.current.detail?.title).toBe("first"));
		await waitFor(() =>
			expect(listeners["workflow-execution-changed"]?.length).toBe(1),
		);

		act(() => {
			listeners["workflow-execution-changed"][0]({
				payload: { worktreePath: "/repo" } as never,
			});
		});
		await waitFor(() =>
			expect(result.current.detail?.title).toBe("workflow refresh"),
		);
	});

	it("reloads for a matching agent session event", async () => {
		responses.push(detail("node", "first"), detail("node", "activity refresh"));
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "node" }),
		);
		await waitFor(() => expect(result.current.detail?.title).toBe("first"));
		await waitFor(() =>
			expect(listeners["agent-session-changed"]?.length).toBe(1),
		);

		act(() => {
			listeners["agent-session-changed"][0]({
				payload: { worktreePath: "/repo" } as never,
			});
		});

		await waitFor(() =>
			expect(result.current.detail?.title).toBe("activity refresh"),
		);
	});

	it("reloads for an unscoped agent session event", async () => {
		responses.push(detail("node", "first"), detail("node", "unscoped refresh"));
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "node" }),
		);
		await waitFor(() => expect(result.current.detail?.title).toBe("first"));

		act(() => {
			listeners["agent-session-changed"][0]({ payload: {} as never });
		});

		await waitFor(() =>
			expect(result.current.detail?.title).toBe("unscoped refresh"),
		);
	});

	it("does not reload for an agent session event from another Worktree", async () => {
		responses.push(detail("node", "first"), detail("node", "unexpected"));
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "node" }),
		);
		await waitFor(() => expect(result.current.detail?.title).toBe("first"));

		act(() => {
			listeners["agent-session-changed"][0]({
				payload: { worktreePath: "/other" } as never,
			});
		});
		await act(async () => {
			await Promise.resolve();
		});

		expect(mockInvoke).toHaveBeenCalledTimes(1);
		expect(result.current.detail?.title).toBe("first");
	});

	it("keeps current detail during background refresh", async () => {
		const pending = deferred<WorkspaceNodeDetail | null>();
		responses.push(detail("node", "current"), pending.promise);
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "node" }),
		);
		await waitFor(() => expect(result.current.detail?.title).toBe("current"));

		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo" },
				}),
			);
		});
		expect(result.current.detail?.title).toBe("current");
		expect(result.current.loading).toBe(true);

		await act(async () => {
			pending.resolve(detail("node", "updated"));
			await pending.promise;
		});
		await waitFor(() => expect(result.current.detail?.title).toBe("updated"));
	});

	it("treats an authoritative null refresh as Node removal", async () => {
		responses.push(detail("node", "current"), null);
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "node" }),
		);
		await waitFor(() => expect(result.current.detail?.title).toBe("current"));

		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo" },
				}),
			);
		});

		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.detail).toBeNull();
		expect(result.current.missingNodeId).toBe("node");
		expect(result.current.error).toBeNull();
	});

	it("keeps current detail when a background refresh fails", async () => {
		responses.push(detail("node", "current"));
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId: "node" }),
		);
		await waitFor(() => expect(result.current.detail?.title).toBe("current"));
		mockInvoke.mockRejectedValueOnce(new Error("offline"));

		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo" },
				}),
			);
		});

		await waitFor(() => expect(result.current.error).toBe("offline"));
		expect(result.current.detail?.title).toBe("current");
		expect(result.current.missingNodeId).toBeNull();
	});

	it("ignores an older occurrence response after selecting a newer occurrence", async () => {
		const firstOccurrenceRefresh = deferred<WorkspaceNodeDetail | null>();
		const secondOccurrenceLoad = deferred<WorkspaceNodeDetail | null>();
		responses.push(
			detailWithSession("occurrence-a-1", "A", "session-a-1"),
			firstOccurrenceRefresh.promise,
			secondOccurrenceLoad.promise,
		);
		const { result, rerender } = renderHook(
			({ nodeId }) => useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId }),
			{ initialProps: { nodeId: "occurrence-a-1" } },
		);
		await waitFor(() =>
			expect(result.current.detail?.id).toBe("occurrence-a-1"),
		);

		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo" },
				}),
			);
		});
		rerender({ nodeId: "occurrence-a-2" });

		await act(async () => {
			secondOccurrenceLoad.resolve(
				detailWithSession("occurrence-a-2", "A", "session-a-2"),
			);
			await secondOccurrenceLoad.promise;
		});
		await waitFor(() =>
			expect(result.current.detail?.content).toEqual({
				kind: "session",
				sessionId: "session-a-2",
			}),
		);

		await act(async () => {
			firstOccurrenceRefresh.resolve(
				detailWithSession("occurrence-a-1", "A", "stale-session-a-1"),
			);
			await firstOccurrenceRefresh.promise;
		});
		expect(result.current.detail?.id).toBe("occurrence-a-2");
		expect(result.current.detail?.content).toEqual({
			kind: "session",
			sessionId: "session-a-2",
		});
	});

	it("ignores an older refresh response for the same occurrence", async () => {
		const older = deferred<WorkspaceNodeDetail | null>();
		const latest = deferred<WorkspaceNodeDetail | null>();
		responses.push(
			detailWithSession("occurrence-a-1", "A", "session-a-1"),
			older.promise,
			latest.promise,
		);
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({
				worktreePath: "/repo",
				nodeId: "occurrence-a-1",
			}),
		);
		await waitFor(() =>
			expect(result.current.detail?.content).toEqual({
				kind: "session",
				sessionId: "session-a-1",
			}),
		);

		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo" },
				}),
			);
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo" },
				}),
			);
		});

		await act(async () => {
			latest.resolve(
				detailWithSession("occurrence-a-1", "A", "latest-session-a-1"),
			);
			await latest.promise;
		});
		await waitFor(() =>
			expect(result.current.detail?.content).toEqual({
				kind: "session",
				sessionId: "latest-session-a-1",
			}),
		);

		await act(async () => {
			older.resolve(
				detailWithSession("occurrence-a-1", "A", "stale-session-a-1"),
			);
			await older.promise;
		});
		expect(result.current.detail?.content).toEqual({
			kind: "session",
			sessionId: "latest-session-a-1",
		});
	});

	it("clears stale detail immediately when node selection changes", async () => {
		const pending = deferred<WorkspaceNodeDetail | null>();
		responses.push(detail("first"), pending.promise);
		const { result, rerender } = renderHook(
			({ nodeId }) => useWorkspaceNodeDetail({ worktreePath: "/repo", nodeId }),
			{ initialProps: { nodeId: "first" as string | null } },
		);
		await waitFor(() => expect(result.current.detail?.id).toBe("first"));

		rerender({ nodeId: "second" });
		expect(result.current.detail).toBeNull();
		expect(result.current.loading).toBe(true);

		await act(async () => {
			pending.resolve(detail("second"));
			await pending.promise;
		});
		await waitFor(() => expect(result.current.detail?.id).toBe("second"));
	});

	it("approves through the opaque workspace node command and reloads detail", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "approve_workspace_node") return Promise.resolve(null);
			if (command === "get_workspace_node_detail") {
				return Promise.resolve(detail("node"));
			}
			return Promise.resolve(null);
		});

		const result = await approveWorkspaceNode({
			worktreePath: "/repo",
			nodeId: "node",
		});

		expect(mockInvoke).toHaveBeenNthCalledWith(1, "approve_workspace_node", {
			worktreePath: "/repo",
			nodeId: "node",
		});
		expect(mockInvoke).toHaveBeenNthCalledWith(2, "get_workspace_node_detail", {
			worktreePath: "/repo",
			nodeId: "node",
		});
		expect(result).toEqual(detail("node"));
	});

	it("retries through the opaque workspace node command and reloads detail", async () => {
		mockInvoke.mockImplementation((command: string) => {
			if (command === "retry_workspace_node") return Promise.resolve(null);
			if (command === "get_workspace_node_detail") {
				return Promise.resolve(detail("node"));
			}
			return Promise.resolve(null);
		});

		const result = await retryWorkspaceNode({
			worktreePath: "/repo",
			nodeId: "node",
		});

		expect(mockInvoke).toHaveBeenNthCalledWith(1, "retry_workspace_node", {
			worktreePath: "/repo",
			nodeId: "node",
		});
		expect(mockInvoke).toHaveBeenNthCalledWith(2, "get_workspace_node_detail", {
			worktreePath: "/repo",
			nodeId: "node",
		});
		expect(result).toEqual(detail("node"));
	});
});
