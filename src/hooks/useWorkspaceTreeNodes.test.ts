import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStatus, SessionSummary } from "@/types/session";
import type { WorkflowExecutionChangedPayload } from "@/types/workflow";
import type {
	WorkspaceTreeItem,
	WorkspaceTreeSnapshot,
} from "@/types/workspace-tree";
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

type ListenerMap = Record<string, Array<(event: { payload: never }) => void>>;

function makeNode(id: string): WorkspaceTreeItem {
	return {
		kind: "node",
		id,
		title: id,
		status: "running",
		contentKind: "session",
		capabilities: { canApprove: false, canClose: true },
		updatedAt: 1,
	};
}

function makeSnapshot(
	nodes: WorkspaceTreeItem[],
	preferredNodeId: string | null = null,
): WorkspaceTreeSnapshot {
	return { nodes, preferredNodeId };
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
		last_activity_at: 1,
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
	let treeResponses: Array<
		WorkspaceTreeSnapshot | Promise<WorkspaceTreeSnapshot>
	>;

	beforeEach(() => {
		vi.clearAllMocks();
		listeners = {};
		treeResponses = [];
		mockListClosedSessions.mockResolvedValue([]);
		mockListen.mockImplementation(
			(event: string, listener: (event: { payload: never }) => void) => {
				listeners[event] = [...(listeners[event] ?? []), listener];
				return Promise.resolve(vi.fn());
			},
		);
		mockInvoke.mockImplementation((command: string) => {
			if (command === "list_workspace_worktree_nodes") {
				return Promise.resolve(treeResponses.shift() ?? makeSnapshot([], null));
			}
			if (command === "list_workspace_workflow_history") {
				return Promise.resolve([]);
			}
			return Promise.resolve(null);
		});
	});

	it("loads a recursive snapshot and exposes preferredNodeId", async () => {
		treeResponses.push(makeSnapshot([makeNode("node-1")], "node-1"));
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));

		expect(result.current.loading).toBe(true);
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.nodes).toEqual([makeNode("node-1")]);
		expect(result.current.preferredNodeId).toBe("node-1");
	});

	it("keeps the previous snapshot visible during a background refresh", async () => {
		const pending = deferred<WorkspaceTreeSnapshot>();
		treeResponses.push(makeSnapshot([makeNode("node-1")], "node-1"));
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));

		treeResponses.push(pending.promise);
		act(() => {
			window.dispatchEvent(
				new CustomEvent("workspace-tree-refresh", {
					detail: { worktreePath: "/repo" },
				}),
			);
		});
		await waitForScheduledRefresh();
		expect(result.current.nodes).toEqual([makeNode("node-1")]);

		await act(async () => {
			pending.resolve(makeSnapshot([makeNode("node-2")], "node-2"));
			await pending.promise;
		});
		await waitFor(() =>
			expect(result.current.nodes).toEqual([makeNode("node-2")]),
		);
	});

	it("refreshes for every matching Worktree session status without inspecting opaque ids", async () => {
		treeResponses.push(
			makeSnapshot([makeNode("opaque-node")]),
			makeSnapshot([]),
		);
		renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(countTreeFetches()).toBe(1));
		await waitFor(() =>
			expect(listeners["session-status-changed"]?.length).toBe(1),
		);

		act(() => {
			listeners["session-status-changed"][0]({
				payload: makeStatus({
					chat_session_id: "unrelated-to-node-id",
				}) as never,
			});
		});
		await waitForScheduledRefresh();
		expect(countTreeFetches()).toBe(2);
	});

	it("ignores session and workflow events for another Worktree", async () => {
		treeResponses.push(makeSnapshot([]));
		renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(countTreeFetches()).toBe(1));
		await waitFor(() =>
			expect(listeners["workflow-execution-changed"]?.length).toBe(1),
		);

		act(() => {
			listeners["session-status-changed"][0]({
				payload: makeStatus({ worktree_path: "/other" }) as never,
			});
			listeners["workflow-execution-changed"][0]({
				payload: {
					worktreePath: "/other",
				} as WorkflowExecutionChangedPayload as never,
			});
		});
		await waitForScheduledRefresh();
		expect(countTreeFetches()).toBe(1);
	});

	it("does not filter closed sessions by workflow metadata in frontend", async () => {
		const closed = {
			id: "closed-workflow-session",
			worktreePath: "/repo",
			state: "closed",
			createdAt: 1,
			updatedAt: 2,
			firstMessage: "closed",
			messageCount: 1,
			workflowNodeSession: true,
		} as SessionSummary;
		mockListClosedSessions.mockResolvedValue([closed]);
		treeResponses.push(makeSnapshot([]));

		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.closedSessions).toEqual([closed]);
	});

	it("ignores an older Worktree response after the path changes", async () => {
		const oldResponse = deferred<WorkspaceTreeSnapshot>();
		treeResponses.push(oldResponse.promise, makeSnapshot([makeNode("new")]));
		const { result, rerender } = renderHook(
			({ path }) => useWorkspaceTreeNodes(path),
			{ initialProps: { path: "/old" as string | null } },
		);

		rerender({ path: "/new" });
		await waitFor(() =>
			expect(result.current.nodes).toEqual([makeNode("new")]),
		);
		await act(async () => {
			oldResponse.resolve(makeSnapshot([makeNode("old")]));
			await oldResponse.promise;
		});
		expect(result.current.nodes).toEqual([makeNode("new")]);
	});
});
