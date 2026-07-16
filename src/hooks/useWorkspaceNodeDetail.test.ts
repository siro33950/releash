import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStatus } from "@/types/session";
import type { WorkspaceNodeDetail } from "@/types/workspace-tree";
import {
	approveWorkspaceNode,
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
		capabilities: { canApprove: false, canClose: false },
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
		const sessionListenerReady = deferred<() => void>();
		let currentDetail = detailWithSession(
			"node",
			"before session attach",
			"session-before",
		);

		mockListen.mockImplementation((event: string, listener: Listener) => {
			listeners[event] = [...(listeners[event] ?? []), listener];
			return event === "workflow-execution-changed"
				? workflowListenerReady.promise
				: sessionListenerReady.promise;
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

		await waitFor(() => expect(mockListen).toHaveBeenCalledTimes(1));
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
		await waitFor(() => expect(mockListen).toHaveBeenCalledTimes(2));
		expect(mockInvoke).not.toHaveBeenCalled();

		await act(async () => {
			sessionListenerReady.resolve(vi.fn());
			await sessionListenerReady.promise;
		});

		await waitFor(() => expect(result.current.detail).toEqual(currentDetail));
		expect(mockInvoke).toHaveBeenCalledTimes(1);
	});

	it("reloads for matching Worktree workflow and session events", async () => {
		responses.push(
			detail("node", "first"),
			detail("node", "workflow refresh"),
			detail("node", "session refresh"),
		);
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

		act(() => {
			listeners["session-status-changed"][0]({
				payload: { worktree_path: "/repo" } as SessionStatus as never,
			});
		});
		await waitFor(() =>
			expect(result.current.detail?.title).toBe("session refresh"),
		);
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

	it("ignores an older retry response that resolves after the latest detail", async () => {
		const older = deferred<WorkspaceNodeDetail | null>();
		const latest = deferred<WorkspaceNodeDetail | null>();
		responses.push(
			detailWithSession("stable-node", "attempt 1", "session-attempt-1"),
			older.promise,
			latest.promise,
		);
		const { result } = renderHook(() =>
			useWorkspaceNodeDetail({
				worktreePath: "/repo",
				nodeId: "stable-node",
			}),
		);
		await waitFor(() => expect(result.current.detail?.title).toBe("attempt 1"));

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
				detailWithSession("stable-node", "latest retry", "session-attempt-3"),
			);
			await latest.promise;
		});
		await waitFor(() =>
			expect(result.current.detail?.content).toEqual({
				kind: "session",
				sessionId: "session-attempt-3",
			}),
		);

		await act(async () => {
			older.resolve(
				detailWithSession("stable-node", "stale retry", "session-attempt-2"),
			);
			await older.promise;
		});
		expect(result.current.detail?.title).toBe("latest retry");
		expect(result.current.detail?.content).toEqual({
			kind: "session",
			sessionId: "session-attempt-3",
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
});
