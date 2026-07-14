import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStatus } from "@/types/session";
import type {
	WorkflowExecution,
	WorkflowExecutionChangedPayload,
} from "@/types/workflow";
import type { WorkspaceTreeNode } from "@/types/workspace-tree";
import { useWorkspaceTreeNodes } from "./useWorkspaceTreeNodes";

const mockInvoke = vi.fn();
const mockListen = vi.fn();
const mockListClosedSessions = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

vi.mock("@/hooks/useSessionStore", () => ({
	listClosedSessions: (...args: unknown[]) => mockListClosedSessions(...args),
}));

type ListenerMap = Record<string, Array<(event: { payload: unknown }) => void>>;

function makeSessionNode(id: string): WorkspaceTreeNode {
	return {
		kind: "session",
		id,
		worktreePath: "/repo",
		title: id,
		state: "active",
		updatedAt: 1_000,
		workflowNodeSession: false,
		agentState: "running",
	};
}

function makeStatus(overrides: Partial<SessionStatus> = {}): SessionStatus {
	return {
		chat_session_id: "session-1",
		worktree_id: "/repo",
		worktree_path: "/repo",
		pty_id: null,
		agent_state: "running",
		turn_phase: "streaming",
		session_state: "active",
		pending_permission: false,
		last_activity_at: 1_000,
		...overrides,
	};
}

function makeWorkflowExecution(
	overrides: Partial<WorkflowExecution> = {},
): WorkflowExecution {
	return {
		id: "execution-1",
		workflowName: "workflow",
		status: "running",
		currentNode: "review",
		worktreePath: "/repo",
		createdFrom: "desktop_ui",
		startedAt: 1_000,
		updatedAt: 1_000,
		completedAt: null,
		errorReason: null,
		totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
		nodeExecutions: [],
		artifacts: [],
		fanouts: [],
		approvalTarget: null,
		...overrides,
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

function countTreeFetches(): number {
	return mockInvoke.mock.calls.filter(
		([command]) => command === "list_workspace_worktree_nodes",
	).length;
}

async function waitForScheduledRefresh() {
	await new Promise((resolve) => window.setTimeout(resolve, 100));
}

describe("useWorkspaceTreeNodes", () => {
	let listeners: ListenerMap;
	let treeResponses: Array<WorkspaceTreeNode[] | Promise<WorkspaceTreeNode[]>>;

	beforeEach(() => {
		vi.clearAllMocks();
		listeners = {};
		treeResponses = [];
		mockListClosedSessions.mockResolvedValue([]);
		mockListen.mockImplementation(
			(event: string, fn: (event: { payload: unknown }) => void) => {
				listeners[event] = [...(listeners[event] ?? []), fn];
				return Promise.resolve(vi.fn());
			},
		);
		mockInvoke.mockImplementation((command: string) => {
			if (command === "list_workspace_worktree_nodes") {
				const response = treeResponses.shift() ?? [];
				return Promise.resolve(response);
			}
			if (command === "list_workspace_workflow_history") {
				return Promise.resolve([]);
			}
			return Promise.resolve(null);
		});
	});

	it("marks a valid Worktree as loading before the first fetch completes", async () => {
		const pending = deferred<WorkspaceTreeNode[]>();
		treeResponses.push(pending.promise);

		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));

		expect(result.current.loading).toBe(true);

		await act(async () => {
			pending.resolve([]);
			await pending.promise;
		});

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});
	});

	it("marks a changed Worktree path as loading until that path has loaded", async () => {
		const initial = [makeSessionNode("session-1")];
		treeResponses.push(initial);

		const { result, rerender } = renderHook(
			({ worktreePath }: { worktreePath: string }) =>
				useWorkspaceTreeNodes(worktreePath),
			{ initialProps: { worktreePath: "/repo" } },
		);

		await waitFor(() => {
			expect(result.current.nodes).toEqual(initial);
		});
		expect(result.current.loading).toBe(false);

		const pending = deferred<WorkspaceTreeNode[]>();
		treeResponses.push(pending.promise);

		rerender({ worktreePath: "/repo/next" });

		expect(result.current.loading).toBe(true);

		await act(async () => {
			pending.resolve([]);
			await pending.promise;
		});

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});
		expect(result.current.nodes).toEqual([]);
	});

	it("unsubscribes listeners when cleanup runs after setup completes", async () => {
		const unlistenStatus = vi.fn();
		const unlistenWorkflow = vi.fn();
		mockListen.mockImplementation(
			(event: string, fn: (event: { payload: unknown }) => void) => {
				listeners[event] = [...(listeners[event] ?? []), fn];
				if (event === "session-status-changed") {
					return Promise.resolve(unlistenStatus);
				}
				if (event === "workflow-execution-changed") {
					return Promise.resolve(unlistenWorkflow);
				}
				return Promise.resolve(vi.fn());
			},
		);

		const { unmount } = renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledTimes(2);
		});

		unmount();

		expect(unlistenStatus).toHaveBeenCalledTimes(1);
		expect(unlistenWorkflow).toHaveBeenCalledTimes(1);
	});

	it("unsubscribes the status listener if cleanup runs before setup completes", async () => {
		const pendingStatus = deferred<() => void>();
		const unlistenStatus = vi.fn();
		mockListen.mockImplementationOnce(() => pendingStatus.promise);

		const { unmount } = renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledTimes(1);
		});

		unmount();

		await act(async () => {
			pendingStatus.resolve(unlistenStatus);
			await pendingStatus.promise;
		});

		expect(unlistenStatus).toHaveBeenCalledTimes(1);
		expect(mockListen).toHaveBeenCalledTimes(1);
	});

	it("unsubscribes the workflow listener if cleanup runs before it resolves", async () => {
		const pendingWorkflow = deferred<() => void>();
		const unlistenStatus = vi.fn();
		const unlistenWorkflow = vi.fn();
		mockListen.mockImplementation(
			(event: string, fn: (event: { payload: unknown }) => void) => {
				listeners[event] = [...(listeners[event] ?? []), fn];
				if (event === "session-status-changed") {
					return Promise.resolve(unlistenStatus);
				}
				if (event === "workflow-execution-changed") {
					return pendingWorkflow.promise;
				}
				return Promise.resolve(vi.fn());
			},
		);

		const { unmount } = renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(mockListen).toHaveBeenCalledTimes(2);
		});

		unmount();

		expect(unlistenStatus).toHaveBeenCalledTimes(1);
		expect(unlistenWorkflow).not.toHaveBeenCalled();

		await act(async () => {
			pendingWorkflow.resolve(unlistenWorkflow);
			await pendingWorkflow.promise;
		});

		expect(unlistenWorkflow).toHaveBeenCalledTimes(1);
	});

	it("keeps existing nodes visible during background refresh", async () => {
		const initial = [makeSessionNode("session-1")];
		const next = [makeSessionNode("session-1"), makeSessionNode("session-2")];
		treeResponses.push(initial);

		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(result.current.nodes).toEqual(initial);
		});

		const pending = deferred<WorkspaceTreeNode[]>();
		treeResponses.push(pending.promise);

		let refreshPromise!: Promise<void>;
		act(() => {
			refreshPromise = result.current.refresh();
		});

		expect(result.current.loading).toBe(false);
		expect(result.current.nodes).toEqual(initial);

		await act(async () => {
			pending.resolve(next);
			await refreshPromise;
		});

		expect(result.current.nodes).toEqual(next);
	});

	it("keeps existing nodes when a background refresh fails", async () => {
		const initial = [makeSessionNode("session-1")];
		treeResponses.push(initial);

		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(result.current.nodes).toEqual(initial);
		});

		treeResponses.push(Promise.reject(new Error("boom")));

		await act(async () => {
			await result.current.refresh();
		});

		expect(result.current.loading).toBe(false);
		expect(result.current.nodes).toEqual(initial);
		expect(result.current.error).toBe("Error: boom");
	});

	it("does not refresh the tree for known session status updates", async () => {
		treeResponses.push([makeSessionNode("session-1")]);

		renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(countTreeFetches()).toBe(1);
		});
		await waitFor(() => {
			expect(listeners["session-status-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["session-status-changed"]?.[0]?.({
				payload: makeStatus({ chat_session_id: "session-1" }),
			});
			await waitForScheduledRefresh();
		});

		expect(countTreeFetches()).toBe(1);
	});

	it("does not refresh the tree for known workflow node session updates", async () => {
		treeResponses.push([
			{
				kind: "workflow",
				executionId: "execution-1",
				worktreePath: "/repo",
				workflowName: "workflow",
				title: "workflow",
				status: "running",
				canStop: true,
				updatedAt: 1_000,
				nodeExecutions: [
					{
						kind: "node",
						nodeExecutionId: "node-review-1",
						executionId: "execution-1",
						worktreePath: "/repo",
						title: "review",
						nodeName: "review",
						status: "running",
						nodeKind: "session",
						updatedAt: 1_000,
						attempt: 1,
						sessions: [
							{
								kind: "session",
								id: "node-session-1",
								worktreePath: "/repo",
								title: "review",
								state: "active",
								updatedAt: 1_000,
								workflowNodeSession: true,
								nodeExecutionId: "node-review-1",
								nodeName: "review",
								attempt: 1,
							},
						],
					},
				],
			},
		]);

		renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(countTreeFetches()).toBe(1);
		});
		await waitFor(() => {
			expect(listeners["session-status-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["session-status-changed"]?.[0]?.({
				payload: makeStatus({ chat_session_id: "node-session-1" }),
			});
			await waitForScheduledRefresh();
		});

		expect(countTreeFetches()).toBe(1);
	});

	it("refreshes the tree when a new session appears", async () => {
		treeResponses.push([makeSessionNode("session-1")]);

		renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(countTreeFetches()).toBe(1);
		});
		await waitFor(() => {
			expect(listeners["session-status-changed"]?.length).toBe(1);
		});

		treeResponses.push([
			makeSessionNode("session-1"),
			makeSessionNode("session-2"),
		]);
		await act(async () => {
			listeners["session-status-changed"]?.[0]?.({
				payload: makeStatus({ chat_session_id: "session-2" }),
			});
			await waitForScheduledRefresh();
		});

		await waitFor(() => {
			expect(countTreeFetches()).toBe(2);
		});
	});

	it("refreshes the tree when a known workflow reaches a terminal state", async () => {
		treeResponses.push([
			{
				kind: "workflow",
				executionId: "execution-1",
				worktreePath: "/repo",
				workflowName: "workflow",
				title: "workflow",
				status: "running",
				canStop: true,
				updatedAt: 1_000,
				nodeExecutions: [],
			},
		]);

		renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(countTreeFetches()).toBe(1);
		});
		await waitFor(() => {
			expect(listeners["workflow-execution-changed"]?.length).toBe(1);
		});

		treeResponses.push([]);
		const payload: WorkflowExecutionChangedPayload = {
			worktreePath: "/repo",
			workflowExecution: makeWorkflowExecution({
				status: "completed",
			}),
		};
		await act(async () => {
			listeners["workflow-execution-changed"]?.[0]?.({ payload });
			await waitForScheduledRefresh();
		});

		await waitFor(() => {
			expect(countTreeFetches()).toBe(2);
		});
	});

	it("refreshes the tree when a known workflow changes non-terminal state", async () => {
		treeResponses.push([
			{
				kind: "workflow",
				executionId: "execution-1",
				worktreePath: "/repo",
				workflowName: "workflow",
				title: "workflow",
				status: "running",
				canStop: true,
				updatedAt: 1_000,
				nodeExecutions: [],
			},
		]);

		renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(countTreeFetches()).toBe(1);
		});
		await waitFor(() => {
			expect(listeners["workflow-execution-changed"]?.length).toBe(1);
		});

		treeResponses.push([
			{
				kind: "workflow",
				executionId: "execution-1",
				worktreePath: "/repo",
				workflowName: "workflow",
				title: "workflow",
				status: "waiting",
				canStop: true,
				updatedAt: 2_000,
				nodeExecutions: [],
			},
		]);
		const payload: WorkflowExecutionChangedPayload = {
			worktreePath: "/repo",
			workflowExecution: makeWorkflowExecution({
				status: "waiting_approval",
			}),
		};
		await act(async () => {
			listeners["workflow-execution-changed"]?.[0]?.({ payload });
			await waitForScheduledRefresh();
		});

		await waitFor(() => {
			expect(countTreeFetches()).toBe(2);
		});
	});
});
