import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStatus } from "@/types/session";
import type { WorkflowState, WorkflowStatePayload } from "@/types/workflow";
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
		workflowStepSession: false,
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

function makeWorkflowState(
	overrides: Partial<WorkflowState> = {},
): WorkflowState {
	return {
		executionId: "run-1",
		workflowName: "workflow",
		state: { type: "running" },
		currentStepIndex: 0,
		currentStepName: "step",
		totalSteps: 1,
		stepHistory: [],
		stepExecutionCounts: {},
		stepOutputs: {},
		workflowDefinition: {
			name: "workflow",
			description: "",
			builtin: false,
			nodes: [],
		},
		totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
		stepStates: {},
		startedAt: 1_000,
		updatedAt: 1_000,
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

	it("unsubscribes listeners when cleanup runs after setup completes", async () => {
		const unlistenStatus = vi.fn();
		const unlistenWorkflow = vi.fn();
		mockListen.mockImplementation(
			(event: string, fn: (event: { payload: unknown }) => void) => {
				listeners[event] = [...(listeners[event] ?? []), fn];
				if (event === "session-status-changed") {
					return Promise.resolve(unlistenStatus);
				}
				if (event === "workflow-state-changed") {
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
				if (event === "workflow-state-changed") {
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

	it("does not refresh the tree for known workflow step session status updates", async () => {
		treeResponses.push([
			{
				kind: "workflow",
				runId: "run-1",
				worktreePath: "/repo",
				workflowName: "workflow",
				title: "workflow",
				status: "running",
				updatedAt: 1_000,
				steps: [
					{
						kind: "step",
						id: "run-1:review:1",
						runId: "run-1",
						worktreePath: "/repo",
						title: "review",
						status: "running",
						stepType: "agent",
						updatedAt: 1_000,
						runIndex: 1,
						sessions: [
							{
								kind: "session",
								id: "step-session-1",
								worktreePath: "/repo",
								title: "review",
								state: "active",
								updatedAt: 1_000,
								workflowStepSession: true,
								stepName: "review",
								runIndex: 1,
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
				payload: makeStatus({ chat_session_id: "step-session-1" }),
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
				runId: "run-1",
				worktreePath: "/repo",
				workflowName: "workflow",
				title: "workflow",
				status: "running",
				updatedAt: 1_000,
				steps: [],
			},
		]);

		renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(countTreeFetches()).toBe(1);
		});
		await waitFor(() => {
			expect(listeners["workflow-state-changed"]?.length).toBe(1);
		});

		treeResponses.push([]);
		const payload: WorkflowStatePayload = {
			worktreePath: "/repo",
			workflowState: makeWorkflowState({
				state: { type: "completed" },
			}),
		};
		await act(async () => {
			listeners["workflow-state-changed"]?.[0]?.({ payload });
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
				runId: "run-1",
				worktreePath: "/repo",
				workflowName: "workflow",
				title: "workflow",
				status: "running",
				updatedAt: 1_000,
				steps: [],
			},
		]);

		renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => {
			expect(countTreeFetches()).toBe(1);
		});
		await waitFor(() => {
			expect(listeners["workflow-state-changed"]?.length).toBe(1);
		});

		treeResponses.push([
			{
				kind: "workflow",
				runId: "run-1",
				worktreePath: "/repo",
				workflowName: "workflow",
				title: "workflow",
				status: "waiting_approval",
				updatedAt: 2_000,
				steps: [],
			},
		]);
		const payload: WorkflowStatePayload = {
			worktreePath: "/repo",
			workflowState: makeWorkflowState({
				state: { type: "waiting_approval" },
			}),
		};
		await act(async () => {
			listeners["workflow-state-changed"]?.[0]?.({ payload });
			await waitForScheduledRefresh();
		});

		await waitFor(() => {
			expect(countTreeFetches()).toBe(2);
		});
	});
});
