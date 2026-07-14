import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStatus } from "@/types/session";
import type {
	WorkflowExecution,
	WorkflowExecutionChangedPayload,
} from "@/types/workflow";
import type { WorkspaceWorkflowNodeDetail } from "@/types/workspace-tree";
import {
	submitWorkspaceWorkflowNodeAction,
	useWorkspaceWorkflowNodeDetail,
} from "./useWorkspaceWorkflowNodeDetail";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

type ListenerMap = Record<string, Array<(event: { payload: unknown }) => void>>;

function nodeDetail(
	overrides: Partial<WorkspaceWorkflowNodeDetail> = {},
): WorkspaceWorkflowNodeDetail {
	return {
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
				id: "session-1",
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
		...overrides,
	};
}

function workflowExecution(
	overrides: Partial<WorkflowExecution> = {},
): WorkflowExecution {
	return {
		id: "execution-1",
		workflowName: "workflow",
		status: "waiting_approval",
		currentNode: "review",
		worktreePath: "/repo",
		createdFrom: "desktop_ui",
		startedAt: 1_000,
		updatedAt: 2_000,
		completedAt: null,
		errorReason: null,
		totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
		nodeExecutions: [],
		artifacts: [],
		fanouts: [],
		approvalTarget: {
			nodeExecutionId: "node-review-1",
			nodeName: "review",
			sessionId: "session-1",
		},
		...overrides,
	};
}

function executionPayload(
	overrides: Partial<WorkflowExecutionChangedPayload> = {},
): WorkflowExecutionChangedPayload {
	return {
		worktreePath: "/repo",
		workflowExecution: workflowExecution(),
		...overrides,
	};
}

function sessionStatus(overrides: Partial<SessionStatus> = {}): SessionStatus {
	return {
		chat_session_id: "session-1",
		worktree_id: "/repo",
		worktree_path: "/repo",
		pty_id: null,
		agent_state: "waiting",
		turn_phase: "idle",
		session_state: "done",
		pending_permission: false,
		last_activity_at: 2_000,
		...overrides,
	};
}

describe("useWorkspaceWorkflowNodeDetail", () => {
	let listeners: ListenerMap;
	let responses: Array<
		| WorkspaceWorkflowNodeDetail
		| Promise<WorkspaceWorkflowNodeDetail | null>
		| null
	>;

	beforeEach(() => {
		vi.clearAllMocks();
		listeners = {};
		responses = [];
		mockListen.mockImplementation(
			(event: string, listener: (event: { payload: unknown }) => void) => {
				listeners[event] = [...(listeners[event] ?? []), listener];
				return Promise.resolve(vi.fn());
			},
		);
		mockInvoke.mockImplementation(() => {
			const response = responses.shift() ?? nodeDetail();
			return Promise.resolve(response);
		});
	});

	it("reloads the selected node detail when its execution changes", async () => {
		responses.push(nodeDetail(), nodeDetail({ status: "waiting" }));
		const { result } = renderHook(() =>
			useWorkspaceWorkflowNodeDetail({
				worktreePath: "/repo",
				executionId: "execution-1",
				nodeExecutionId: "node-review-1",
			}),
		);

		await waitFor(() => {
			expect(result.current.detail?.status).toBe("running");
		});
		await waitFor(() => {
			expect(listeners["workflow-execution-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["workflow-execution-changed"]?.[0]?.({
				payload: executionPayload(),
			});
		});

		await waitFor(() => {
			expect(result.current.detail?.status).toBe("waiting");
		});
		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("keeps the displayed node detail when a refresh returns null", async () => {
		responses.push(nodeDetail(), null);
		const { result } = renderHook(() =>
			useWorkspaceWorkflowNodeDetail({
				worktreePath: "/repo",
				executionId: "execution-1",
				nodeExecutionId: "node-review-1",
			}),
		);

		await waitFor(() => {
			expect(result.current.detail?.title).toBe("review");
		});
		await waitFor(() => {
			expect(listeners["workflow-execution-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["workflow-execution-changed"]?.[0]?.({
				payload: executionPayload(),
			});
		});

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});
		expect(result.current.detail?.title).toBe("review");
		expect(result.current.error).toBeNull();
		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("ignores workflow changes for another execution", async () => {
		responses.push(nodeDetail());
		renderHook(() =>
			useWorkspaceWorkflowNodeDetail({
				worktreePath: "/repo",
				executionId: "execution-1",
				nodeExecutionId: "node-review-1",
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});
		await waitFor(() => {
			expect(listeners["workflow-execution-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["workflow-execution-changed"]?.[0]?.({
				payload: executionPayload({
					workflowExecution: workflowExecution({ id: "execution-2" }),
				}),
			});
		});

		expect(mockInvoke).toHaveBeenCalledTimes(1);
	});

	it("reloads when a displayed node session changes state", async () => {
		responses.push(nodeDetail(), nodeDetail({ updatedAt: 2_000 }));
		const { result } = renderHook(() =>
			useWorkspaceWorkflowNodeDetail({
				worktreePath: "/repo",
				executionId: "execution-1",
				nodeExecutionId: "node-review-1",
			}),
		);

		await waitFor(() => {
			expect(result.current.detail?.updatedAt).toBe(1_000);
		});
		await waitFor(() => {
			expect(listeners["session-status-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["session-status-changed"]?.[0]?.({
				payload: sessionStatus(),
			});
		});

		await waitFor(() => {
			expect(result.current.detail?.updatedAt).toBe(2_000);
		});
	});

	it("reloads when a hidden node session is closed", async () => {
		responses.push(
			nodeDetail({ sessions: [] }),
			nodeDetail({
				sessions: [
					{
						...nodeDetail().sessions[0],
						id: "closed-node-session",
						state: "closed",
					},
				],
			}),
		);
		const { result } = renderHook(() =>
			useWorkspaceWorkflowNodeDetail({
				worktreePath: "/repo",
				executionId: "execution-1",
				nodeExecutionId: "node-review-1",
			}),
		);

		await waitFor(() => {
			expect(result.current.detail?.sessions).toHaveLength(0);
		});
		await waitFor(() => {
			expect(listeners["session-status-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["session-status-changed"]?.[0]?.({
				payload: sessionStatus({
					chat_session_id: "closed-node-session",
					session_state: "closed",
				}),
			});
		});

		await waitFor(() => {
			expect(result.current.detail?.sessions[0]?.id).toBe(
				"closed-node-session",
			);
		});
		expect(result.current.detail?.sessions[0]?.state).toBe("closed");
		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("clears stale detail while a newly selected node is loading", async () => {
		let resolveNext: (value: WorkspaceWorkflowNodeDetail | null) => void =
			() => {};
		responses.push(
			nodeDetail({ nodeExecutionId: "node-review-1", title: "review" }),
			new Promise<WorkspaceWorkflowNodeDetail | null>((resolve) => {
				resolveNext = resolve;
			}),
		);
		const { result, rerender } = renderHook(
			({ executionId, nodeExecutionId }) =>
				useWorkspaceWorkflowNodeDetail({
					worktreePath: "/repo",
					executionId,
					nodeExecutionId,
				}),
			{
				initialProps: {
					executionId: "execution-1",
					nodeExecutionId: "node-review-1",
				},
			},
		);

		await waitFor(() => {
			expect(result.current.detail?.title).toBe("review");
		});

		rerender({
			executionId: "execution-1",
			nodeExecutionId: "node-build-1",
		});

		await waitFor(() => {
			expect(result.current.loading).toBe(true);
		});
		expect(result.current.detail).toBeNull();

		await act(async () => {
			resolveNext(
				nodeDetail({ nodeExecutionId: "node-build-1", title: "build" }),
			);
		});

		await waitFor(() => {
			expect(result.current.detail?.title).toBe("build");
		});
	});

	it("does not restore stale detail when a newly selected node fails", async () => {
		let rejectNext: (error: Error) => void = () => {};
		responses.push(
			nodeDetail({ nodeExecutionId: "node-review-1", title: "review" }),
			new Promise<WorkspaceWorkflowNodeDetail | null>((_, reject) => {
				rejectNext = reject;
			}),
		);
		const { result, rerender } = renderHook(
			({ executionId, nodeExecutionId }) =>
				useWorkspaceWorkflowNodeDetail({
					worktreePath: "/repo",
					executionId,
					nodeExecutionId,
				}),
			{
				initialProps: {
					executionId: "execution-1",
					nodeExecutionId: "node-review-1",
				},
			},
		);

		await waitFor(() => {
			expect(result.current.detail?.title).toBe("review");
		});

		rerender({
			executionId: "execution-1",
			nodeExecutionId: "node-build-1",
		});
		await act(async () => {
			rejectNext(new Error("detail failed"));
		});

		await waitFor(() => {
			expect(result.current.error).toBe("detail failed");
		});
		expect(result.current.detail).toBeNull();
	});

	it("submits approval and returns the reloaded node detail", async () => {
		const reloaded = nodeDetail({ status: "completed" });
		const refreshEvents: Array<CustomEvent<{ worktreePath?: string }>> = [];
		const onRefresh = (event: Event) => {
			refreshEvents.push(event as CustomEvent<{ worktreePath?: string }>);
		};
		window.addEventListener("workspace-tree-refresh", onRefresh);
		mockInvoke.mockResolvedValueOnce(undefined).mockResolvedValueOnce(reloaded);

		try {
			const result = await submitWorkspaceWorkflowNodeAction({
				worktreePath: "/repo",
				executionId: "execution-1",
				nodeExecutionId: "node-review-1",
				nodeName: "review",
			});

			expect(mockInvoke).toHaveBeenNthCalledWith(1, "approve_workflow_node", {
				args: {
					executionId: "execution-1",
					nodeName: "review",
					nodeExecutionId: "node-review-1",
					comment: null,
				},
			});
			expect(refreshEvents).toHaveLength(1);
			expect(refreshEvents[0].detail).toEqual({ worktreePath: "/repo" });
			expect(mockInvoke).toHaveBeenNthCalledWith(
				2,
				"get_workspace_workflow_node_detail",
				{
					worktreePath: "/repo",
					executionId: "execution-1",
					nodeExecutionId: "node-review-1",
				},
			);
			expect(result).toBe(reloaded);
		} finally {
			window.removeEventListener("workspace-tree-refresh", onRefresh);
		}
	});
});
