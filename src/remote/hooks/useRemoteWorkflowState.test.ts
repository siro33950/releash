import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type { WsMessage } from "@/types/protocol";
import type { WorkflowState } from "@/types/workflow";
import { useRemoteWorkflowState } from "./useRemoteWorkflowState";

const makeState = (overrides: Partial<WorkflowState> = {}): WorkflowState => ({
	executionId: "exec-1",
	workflowName: "test-wf",
	state: { type: "running" },
	currentStepIndex: 0,
	currentStepName: "plan",
	totalSteps: 2,
	stepHistory: [],
	stepExecutionCounts: {},
	workflowDefinition: {
		name: "test-wf",
		description: "",
		builtin: false,
		steps: [],
	},
	totalTokenUsage: { inputTokens: 0, outputTokens: 0 },
	stepStates: {},
	startedAt: 1000,
	updatedAt: 1000,
	...overrides,
});

describe("useRemoteWorkflowState", () => {
	it("returns null initially", () => {
		const subscribe = vi.fn(() => vi.fn());
		const { result } = renderHook(() =>
			useRemoteWorkflowState({ subscribe, selectedWorktree: "/repo" }),
		);
		expect(result.current.workflowState).toBeNull();
	});

	it("updates state on workflow_state_sync message for matching worktree", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});

		const { result } = renderHook(() =>
			useRemoteWorkflowState({ subscribe, selectedWorktree: "/repo" }),
		);

		const state = makeState();
		act(() => {
			handler?.({
				type: "workflow_state_sync",
				payload: { worktreePath: "/repo", workflowState: state },
			});
		});

		expect(result.current.workflowState).toEqual(state);
	});

	it("ignores workflow_state_sync for different worktree", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});

		const { result } = renderHook(() =>
			useRemoteWorkflowState({ subscribe, selectedWorktree: "/repo" }),
		);

		const state = makeState();
		act(() => {
			handler?.({
				type: "workflow_state_sync",
				payload: { worktreePath: "/other-repo", workflowState: state },
			});
		});

		expect(result.current.workflowState).toBeNull();
	});

	it("ignores workflow_state_sync when selectedWorktree is null", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});

		const { result } = renderHook(() =>
			useRemoteWorkflowState({ subscribe, selectedWorktree: null }),
		);

		const state = makeState();
		act(() => {
			handler?.({
				type: "workflow_state_sync",
				payload: { worktreePath: "/repo", workflowState: state },
			});
		});

		expect(result.current.workflowState).toBeNull();
	});

	it("resets state to null when selectedWorktree becomes null", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});

		const { result, rerender } = renderHook(
			({ selectedWorktree }) =>
				useRemoteWorkflowState({ subscribe, selectedWorktree }),
			{ initialProps: { selectedWorktree: "/repo" as string | null } },
		);

		const state = makeState();
		act(() => {
			handler?.({
				type: "workflow_state_sync",
				payload: { worktreePath: "/repo", workflowState: state },
			});
		});
		expect(result.current.workflowState).toEqual(state);

		rerender({ selectedWorktree: null });
		expect(result.current.workflowState).toBeNull();
	});

	it("ignores non-workflow messages", () => {
		let handler: ((msg: WsMessage) => void) | null = null;
		const subscribe = vi.fn((cb: (msg: WsMessage) => void) => {
			handler = cb;
			return vi.fn();
		});

		const { result } = renderHook(() =>
			useRemoteWorkflowState({ subscribe, selectedWorktree: "/repo" }),
		);

		act(() => {
			handler?.({
				type: "error",
				payload: { code: "err", message: "test" },
			});
		});

		expect(result.current.workflowState).toBeNull();
	});

	it("unsubscribes on unmount", () => {
		const unsubscribe = vi.fn();
		const subscribe = vi.fn(() => unsubscribe);

		const { unmount } = renderHook(() =>
			useRemoteWorkflowState({ subscribe, selectedWorktree: "/repo" }),
		);

		unmount();
		expect(unsubscribe).toHaveBeenCalled();
	});
});
