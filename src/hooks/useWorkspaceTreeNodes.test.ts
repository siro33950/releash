import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkflowExecutionChangedPayload } from "@/types/workflow";
import type {
	WorkspaceTreeItem,
	WorkspaceTreeSelectionSnapshot,
	WorkspaceTreeSnapshot,
} from "@/types/workspace-tree";
import { useWorkspaceTreeNodes } from "./useWorkspaceTreeNodes";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));
vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

type ListenerMap = Record<string, Array<(event: { payload: never }) => void>>;

function makeNode(id: string): WorkspaceTreeItem {
	return {
		kind: "node",
		id,
		title: id,
		status: "active",
		contentKind: "session",
		capabilities: {
			canRename: false,
			canApprove: false,
			canRetry: false,
			canClose: true,
		},
		pastAttempts: [],
		pastAttemptsCollapsed: false,
		updatedAt: 1,
	};
}

function makeSnapshot(
	nodes: WorkspaceTreeItem[],
	preferredNodeId: string | null = null,
): WorkspaceTreeSnapshot {
	return { nodes, archivedSessions: [], preferredNodeId };
}

function makeSelectionSnapshot(
	snapshot: WorkspaceTreeSnapshot,
	selectionInSnapshot: boolean,
): WorkspaceTreeSelectionSnapshot {
	return {
		snapshot,
		reconciliation: { selectionInSnapshot },
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

function countInvocations(command: string): number {
	return mockInvoke.mock.calls.filter(([called]) => called === command).length;
}

async function waitForScheduledRefresh() {
	await new Promise((resolve) => window.setTimeout(resolve, 100));
}

describe("useWorkspaceTreeNodes", () => {
	let listeners: ListenerMap;
	let treeResponses: Array<
		WorkspaceTreeSnapshot | Promise<WorkspaceTreeSnapshot>
	>;
	let selectionResponses: Array<
		WorkspaceTreeSelectionSnapshot | Promise<WorkspaceTreeSelectionSnapshot>
	>;

	beforeEach(() => {
		vi.clearAllMocks();
		listeners = {};
		treeResponses = [];
		selectionResponses = [];
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
			if (command === "get_workspace_tree_selection_reconciliation") {
				return Promise.resolve(
					selectionResponses.shift() ??
						makeSelectionSnapshot(makeSnapshot([], null), false),
				);
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
		expect(countInvocations("list_workspace_worktree_nodes")).toBe(1);
		expect(
			countInvocations("get_workspace_tree_selection_reconciliation"),
		).toBe(0);
	});

	it("Agent TUI cutover後はlegacy closed Sessionを読み込まない", async () => {
		treeResponses.push(makeSnapshot([makeNode("node-1")], "node-1"));
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));

		await waitFor(() => expect(result.current.loading).toBe(false));

		expect(countInvocations("list_workspace_worktree_nodes")).toBe(1);
	});

	it("does not refetch tree or history when only selection changes", async () => {
		treeResponses.push(makeSnapshot([makeNode("node-a")], "node-a"));
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));

		act(() => result.current.synchronizeSelectedNodeId("node-a"));
		act(() => result.current.synchronizeSelectedNodeId("node-b"));
		act(() => result.current.synchronizeSelectedNodeId("node-a"));

		expect(countInvocations("list_workspace_worktree_nodes")).toBe(1);
		expect(
			countInvocations("get_workspace_tree_selection_reconciliation"),
		).toBe(0);
		expect(countInvocations("list_workspace_workflow_history")).toBe(1);
	});

	it("starts Archive reconciliation explicitly and commits snapshot and membership together", async () => {
		treeResponses.push(makeSnapshot([makeNode("selected")], "selected"));
		selectionResponses.push(
			makeSelectionSnapshot(
				makeSnapshot([makeNode("replacement")], "replacement"),
				false,
			),
		);
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.synchronizeSelectedNodeId("selected"));

		await act(async () => {
			await result.current.beginArchiveReconciliation("selected");
		});

		expect(mockInvoke).toHaveBeenCalledWith(
			"get_workspace_tree_selection_reconciliation",
			{ worktreePath: "/repo", selectedNodeId: "selected" },
		);
		expect(result.current.nodes).toEqual([makeNode("replacement")]);
		expect(result.current.preferredNodeId).toBe("replacement");
		expect(result.current.reconciliationEvent).toEqual({
			refreshSeq: expect.any(Number),
			requestContext: {
				worktreePath: "/repo",
				selectedNodeId: "selected",
				reconciliationGeneration: expect.any(Number),
			},
			selectionInSnapshot: false,
		});
		const event = result.current.reconciliationEvent;
		expect(event).not.toBeNull();
		if (!event) throw new Error("expected an accepted reconciliation event");
		expect(result.current.isReconciliationEventCurrent(event, "selected")).toBe(
			true,
		);
	});

	it("retains the old snapshot after a failed Archive read and retries on the next refresh", async () => {
		treeResponses.push(
			makeSnapshot([makeNode("selected")], "selected"),
			makeSnapshot([makeNode("later")], "later"),
		);
		const failed = deferred<WorkspaceTreeSelectionSnapshot>();
		selectionResponses.push(failed.promise);
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.synchronizeSelectedNodeId("selected"));

		let firstAttempt!: Promise<unknown>;
		act(() => {
			firstAttempt = result.current.beginArchiveReconciliation("selected");
		});
		await act(async () => {
			failed.reject(new Error("temporary read failure"));
			await firstAttempt;
		});
		expect(result.current.nodes).toEqual([makeNode("selected")]);
		expect(result.current.preferredNodeId).toBe("selected");
		expect(result.current.reconciliationEvent).toBeNull();

		selectionResponses.push(
			makeSelectionSnapshot(
				makeSnapshot([makeNode("fallback")], "fallback"),
				false,
			),
		);
		await act(async () => {
			await result.current.refresh();
		});
		expect(
			countInvocations("get_workspace_tree_selection_reconciliation"),
		).toBe(2);
		expect(result.current.nodes).toEqual([makeNode("fallback")]);
		expect(result.current.reconciliationEvent?.selectionInSnapshot).toBe(false);

		await act(async () => {
			await result.current.refresh();
		});
		expect(
			countInvocations("get_workspace_tree_selection_reconciliation"),
		).toBe(2);
		expect(countInvocations("list_workspace_worktree_nodes")).toBe(2);
		expect(result.current.nodes).toEqual([makeNode("later")]);
	});

	it("keeps a selected Node when the successful Archive snapshot still contains it", async () => {
		treeResponses.push(makeSnapshot([makeNode("selected")], "selected"));
		selectionResponses.push(
			makeSelectionSnapshot(
				makeSnapshot([makeNode("selected")], "selected"),
				true,
			),
		);
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.synchronizeSelectedNodeId("selected"));

		await act(async () => {
			await result.current.beginArchiveReconciliation("selected");
		});

		expect(result.current.reconciliationEvent?.selectionInSnapshot).toBe(true);
		expect(result.current.nodes).toEqual([makeNode("selected")]);
	});

	it("invalidates an in-flight Archive response after selection moves, including ABA", async () => {
		treeResponses.push(makeSnapshot([makeNode("selected")], "selected"));
		const oldResponse = deferred<WorkspaceTreeSelectionSnapshot>();
		selectionResponses.push(oldResponse.promise);
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.synchronizeSelectedNodeId("selected"));

		let pending!: Promise<unknown>;
		act(() => {
			pending = result.current.beginArchiveReconciliation("selected");
		});
		act(() => result.current.synchronizeSelectedNodeId("other"));
		act(() => result.current.synchronizeSelectedNodeId("selected"));
		await act(async () => {
			oldResponse.resolve(
				makeSelectionSnapshot(
					makeSnapshot([makeNode("stale")], "stale"),
					false,
				),
			);
			await pending;
		});

		expect(result.current.nodes).toEqual([makeNode("selected")]);
		expect(result.current.reconciliationEvent).toBeNull();
	});

	it("invalidates an in-flight Archive response after the Worktree changes", async () => {
		treeResponses.push(
			makeSnapshot([makeNode("old-selected")], "old-selected"),
			makeSnapshot([makeNode("new-worktree")], "new-worktree"),
		);
		const oldResponse = deferred<WorkspaceTreeSelectionSnapshot>();
		selectionResponses.push(oldResponse.promise);
		const { result, rerender } = renderHook(
			({ path }) => useWorkspaceTreeNodes(path),
			{ initialProps: { path: "/old" as string | null } },
		);
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.synchronizeSelectedNodeId("old-selected"));

		let pending!: Promise<unknown>;
		act(() => {
			pending = result.current.beginArchiveReconciliation("old-selected");
		});
		rerender({ path: "/new" });
		await waitFor(() =>
			expect(result.current.nodes).toEqual([makeNode("new-worktree")]),
		);
		await act(async () => {
			oldResponse.resolve(
				makeSelectionSnapshot(
					makeSnapshot([makeNode("stale-old")], "stale-old"),
					false,
				),
			);
			await pending;
		});

		expect(result.current.nodes).toEqual([makeNode("new-worktree")]);
		expect(result.current.reconciliationEvent).toBeNull();
	});

	it("discards an older reconciliation response after a later refresh starts", async () => {
		treeResponses.push(makeSnapshot([makeNode("selected")], "selected"));
		const oldResponse = deferred<WorkspaceTreeSelectionSnapshot>();
		selectionResponses.push(
			oldResponse.promise,
			makeSelectionSnapshot(
				makeSnapshot([makeNode("latest")], "latest"),
				false,
			),
		);
		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));
		act(() => result.current.synchronizeSelectedNodeId("selected"));

		let older!: Promise<unknown>;
		act(() => {
			older = result.current.beginArchiveReconciliation("selected");
		});
		await act(async () => {
			await result.current.refresh();
		});
		expect(result.current.nodes).toEqual([makeNode("latest")]);
		const latestEvent = result.current.reconciliationEvent;

		await act(async () => {
			oldResponse.resolve(
				makeSelectionSnapshot(
					makeSnapshot([makeNode("stale")], "stale"),
					false,
				),
			);
			await older;
		});
		expect(result.current.nodes).toEqual([makeNode("latest")]);
		expect(result.current.reconciliationEvent).toBe(latestEvent);
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

	it("keeps an empty Sequence branch from the backend snapshot unchanged", async () => {
		const emptyWorkflow: WorkspaceTreeItem = {
			kind: "sequence",
			id: "empty-workflow",
			title: "Empty workflow",
			status: "active",
			workflowCapabilities: {
				canStop: true,
				canResume: false,
				canAbort: true,
				canArchive: false,
			},
			children: [],
			updatedAt: 1,
		};
		treeResponses.push(makeSnapshot([emptyWorkflow], null));

		const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));

		expect(result.current.nodes).toEqual([emptyWorkflow]);
		expect(result.current.preferredNodeId).toBeNull();
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

	it("ignores workflow events for another Worktree", async () => {
		treeResponses.push(makeSnapshot([]));
		renderHook(() => useWorkspaceTreeNodes("/repo"));
		await waitFor(() =>
			expect(countInvocations("list_workspace_worktree_nodes")).toBe(1),
		);
		await waitFor(() =>
			expect(listeners["workflow-execution-changed"]?.length).toBe(1),
		);

		act(() => {
			listeners["workflow-execution-changed"][0]({
				payload: {
					worktreePath: "/other",
				} as WorkflowExecutionChangedPayload as never,
			});
		});
		await waitForScheduledRefresh();
		expect(countInvocations("list_workspace_worktree_nodes")).toBe(1);
	});
});
