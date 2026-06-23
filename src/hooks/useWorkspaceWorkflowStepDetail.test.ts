import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStatus } from "@/types/session";
import type { WorkflowStatePayload } from "@/types/workflow";
import type { WorkspaceWorkflowStepDetail } from "@/types/workspace-tree";
import {
	submitWorkspaceWorkflowStepAction,
	useWorkspaceWorkflowStepDetail,
} from "./useWorkspaceWorkflowStepDetail";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

type ListenerMap = Record<string, Array<(event: { payload: unknown }) => void>>;

function stepDetail(
	overrides: Partial<WorkspaceWorkflowStepDetail> = {},
): WorkspaceWorkflowStepDetail {
	return {
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
				id: "session-1",
				worktreePath: "/repo",
				title: "review",
				state: "active",
				updatedAt: 1_000,
				workflowStepSession: true,
				stepName: "review",
				runIndex: 1,
			},
		],
		...overrides,
	};
}

function workflowPayload(
	overrides: Partial<WorkflowStatePayload> = {},
): WorkflowStatePayload {
	return {
		worktreePath: "/repo",
		workflowState: {
			executionId: "run-1",
			workflowName: "workflow",
			state: { type: "waiting_approval" },
			currentStepIndex: 0,
			currentStepName: "review",
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
			updatedAt: 2_000,
		},
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

describe("useWorkspaceWorkflowStepDetail", () => {
	let listeners: ListenerMap;
	let responses: Array<
		| WorkspaceWorkflowStepDetail
		| Promise<WorkspaceWorkflowStepDetail | null>
		| null
	>;

	beforeEach(() => {
		vi.clearAllMocks();
		listeners = {};
		responses = [];
		mockListen.mockImplementation(
			(event: string, fn: (event: { payload: unknown }) => void) => {
				listeners[event] = [...(listeners[event] ?? []), fn];
				return Promise.resolve(vi.fn());
			},
		);
		mockInvoke.mockImplementation(() => {
			const response = responses.shift() ?? stepDetail();
			return Promise.resolve(response);
		});
	});

	it("reloads the selected Step detail when its workflow state changes", async () => {
		responses.push(stepDetail(), stepDetail({ status: "waiting" }));
		const { result } = renderHook(() =>
			useWorkspaceWorkflowStepDetail({
				worktreePath: "/repo",
				runId: "run-1",
				stepId: "run-1:review:1",
			}),
		);

		await waitFor(() => {
			expect(result.current.detail?.status).toBe("running");
		});
		await waitFor(() => {
			expect(listeners["workflow-state-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["workflow-state-changed"]?.[0]?.({
				payload: workflowPayload(),
			});
		});

		await waitFor(() => {
			expect(result.current.detail?.status).toBe("waiting");
		});
		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("keeps the displayed Step detail when a refresh returns null", async () => {
		responses.push(stepDetail(), null);
		const { result } = renderHook(() =>
			useWorkspaceWorkflowStepDetail({
				worktreePath: "/repo",
				runId: "run-1",
				stepId: "run-1:review:1",
			}),
		);

		await waitFor(() => {
			expect(result.current.detail?.title).toBe("review");
		});
		await waitFor(() => {
			expect(listeners["workflow-state-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["workflow-state-changed"]?.[0]?.({
				payload: workflowPayload(),
			});
		});

		await waitFor(() => {
			expect(result.current.loading).toBe(false);
		});
		expect(result.current.detail?.title).toBe("review");
		expect(result.current.error).toBeNull();
		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("ignores workflow changes for another run", async () => {
		responses.push(stepDetail());
		renderHook(() =>
			useWorkspaceWorkflowStepDetail({
				worktreePath: "/repo",
				runId: "run-1",
				stepId: "run-1:review:1",
			}),
		);

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledTimes(1);
		});
		await waitFor(() => {
			expect(listeners["workflow-state-changed"]?.length).toBe(1);
		});

		await act(async () => {
			listeners["workflow-state-changed"]?.[0]?.({
				payload: workflowPayload({
					workflowState: {
						...workflowPayload().workflowState,
						executionId: "run-2",
					},
				}),
			});
		});

		expect(mockInvoke).toHaveBeenCalledTimes(1);
	});

	it("reloads when a displayed Step session changes state", async () => {
		responses.push(stepDetail(), stepDetail({ updatedAt: 2_000 }));
		const { result } = renderHook(() =>
			useWorkspaceWorkflowStepDetail({
				worktreePath: "/repo",
				runId: "run-1",
				stepId: "run-1:review:1",
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

	it("reloads when a Step session is closed before it is displayed", async () => {
		responses.push(
			stepDetail({ sessions: [] }),
			stepDetail({
				sessions: [
					{
						...stepDetail().sessions[0],
						id: "closed-step-session",
						state: "closed",
					},
				],
			}),
		);
		const { result } = renderHook(() =>
			useWorkspaceWorkflowStepDetail({
				worktreePath: "/repo",
				runId: "run-1",
				stepId: "run-1:review:1",
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
					chat_session_id: "closed-step-session",
					session_state: "closed",
				}),
			});
		});

		await waitFor(() => {
			expect(result.current.detail?.sessions[0]?.id).toBe(
				"closed-step-session",
			);
		});
		expect(result.current.detail?.sessions[0]?.state).toBe("closed");
		expect(mockInvoke).toHaveBeenCalledTimes(2);
	});

	it("clears stale detail while a newly selected Step is loading", async () => {
		let resolveNext: (value: WorkspaceWorkflowStepDetail | null) => void =
			() => {};
		responses.push(
			stepDetail({ id: "run-1:review:1", title: "review" }),
			new Promise<WorkspaceWorkflowStepDetail | null>((resolve) => {
				resolveNext = resolve;
			}),
		);
		const { result, rerender } = renderHook(
			({ runId, stepId }: { runId: string; stepId: string }) =>
				useWorkspaceWorkflowStepDetail({
					worktreePath: "/repo",
					runId,
					stepId,
				}),
			{
				initialProps: {
					runId: "run-1",
					stepId: "run-1:review:1",
				},
			},
		);

		await waitFor(() => {
			expect(result.current.detail?.title).toBe("review");
		});

		rerender({ runId: "run-1", stepId: "run-1:build:1" });

		await waitFor(() => {
			expect(result.current.loading).toBe(true);
		});
		expect(result.current.detail).toBeNull();

		await act(async () => {
			resolveNext(stepDetail({ id: "run-1:build:1", title: "build" }));
		});

		await waitFor(() => {
			expect(result.current.detail?.title).toBe("build");
		});
	});

	it("does not restore stale detail when a newly selected Step fails to load", async () => {
		let rejectNext: (error: Error) => void = () => {};
		responses.push(
			stepDetail({ id: "run-1:review:1", title: "review" }),
			new Promise<WorkspaceWorkflowStepDetail | null>((_, reject) => {
				rejectNext = reject;
			}),
		);
		const { result, rerender } = renderHook(
			({ runId, stepId }: { runId: string; stepId: string }) =>
				useWorkspaceWorkflowStepDetail({
					worktreePath: "/repo",
					runId,
					stepId,
				}),
			{
				initialProps: {
					runId: "run-1",
					stepId: "run-1:review:1",
				},
			},
		);

		await waitFor(() => {
			expect(result.current.detail?.title).toBe("review");
		});

		rerender({ runId: "run-1", stepId: "run-1:build:1" });

		await act(async () => {
			rejectNext(new Error("detail failed"));
		});

		await waitFor(() => {
			expect(result.current.error).toBe("detail failed");
		});
		expect(result.current.detail).toBeNull();
	});

	it("submits approve, refreshes the workspace tree, and returns reloaded detail", async () => {
		const reloaded = stepDetail({ status: "completed" });
		const refreshEvents: Array<CustomEvent<{ worktreePath?: string }>> = [];
		const onRefresh = (event: Event) => {
			refreshEvents.push(event as CustomEvent<{ worktreePath?: string }>);
		};
		window.addEventListener("workspace-tree-refresh", onRefresh);
		mockInvoke.mockResolvedValueOnce(undefined).mockResolvedValueOnce(reloaded);

		try {
			const result = await submitWorkspaceWorkflowStepAction({
				worktreePath: "/repo",
				runId: "run-1",
				stepId: "run-1:review:1",
				stepName: "review",
				action: "approve",
			});

			expect(mockInvoke).toHaveBeenNthCalledWith(1, "approve_workflow_step", {
				runId: "run-1",
				stepName: "review",
				decision: { approve: { comment: null } },
			});
			expect(refreshEvents).toHaveLength(1);
			expect(refreshEvents[0].detail).toEqual({ worktreePath: "/repo" });
			expect(mockInvoke).toHaveBeenNthCalledWith(
				2,
				"get_workspace_workflow_step_detail",
				{
					worktreePath: "/repo",
					runId: "run-1",
					stepId: "run-1:review:1",
				},
			);
			expect(result).toBe(reloaded);
		} finally {
			window.removeEventListener("workspace-tree-refresh", onRefresh);
		}
	});

	it("submits reject with the comment as the decision reason", async () => {
		const reloaded = stepDetail({ status: "failed" });
		const refreshEvents: Array<CustomEvent<{ worktreePath?: string }>> = [];
		const onRefresh = (event: Event) => {
			refreshEvents.push(event as CustomEvent<{ worktreePath?: string }>);
		};
		window.addEventListener("workspace-tree-refresh", onRefresh);
		mockInvoke.mockResolvedValueOnce(undefined).mockResolvedValueOnce(reloaded);

		try {
			const result = await submitWorkspaceWorkflowStepAction({
				worktreePath: "/repo",
				runId: "run-1",
				stepId: "run-1:review:1",
				stepName: "review",
				action: "reject",
				reason: "needs tests",
			});

			expect(mockInvoke).toHaveBeenNthCalledWith(1, "approve_workflow_step", {
				runId: "run-1",
				stepName: "review",
				decision: { reject: { reason: "needs tests" } },
			});
			expect(refreshEvents).toHaveLength(1);
			expect(refreshEvents[0].detail).toEqual({ worktreePath: "/repo" });
			expect(mockInvoke).toHaveBeenNthCalledWith(
				2,
				"get_workspace_workflow_step_detail",
				{
					worktreePath: "/repo",
					runId: "run-1",
					stepId: "run-1:review:1",
				},
			);
			expect(result).toBe(reloaded);
		} finally {
			window.removeEventListener("workspace-tree-refresh", onRefresh);
		}
	});
});
