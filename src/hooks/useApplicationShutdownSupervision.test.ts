import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useApplicationShutdownSupervision } from "./useApplicationShutdownSupervision";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

function defaultInvoke(command: string) {
	switch (command) {
		case "list_pending_application_attempts":
			return Promise.resolve({ entries: [], next_cursor: null });
		case "get_application_shutdown":
			return Promise.resolve({
				type: "current",
				plan: {
					shutdown_id: "plan-1",
					phase: "reconciliation_required",
					outcome: "reconciliation_required",
					actions: [],
				},
			});
		case "get_shutdown_plan":
			return Promise.resolve({
				plan: {},
				targets: [
					{
						ordinal: "0",
						target_key: "target-key-1",
						target_id: "session-1",
						kind: "agent_session",
						effect_identity: "shutdown-target/plan-1/0",
						state: "reconciliation_required",
						observation: {
							type: "exit_coupled_outcome_unknown",
							shutdown_id: "plan-1",
						},
						revision: "2",
						actions: ["retry_same_effect"],
						action_identities: [
							{
								action_id: "shutdown-action-1",
								action: "retry_same_effect",
								origin_revision: "2",
							},
						],
					},
				],
				next_cursor: null,
			});
		default:
			return Promise.resolve(undefined);
	}
}

describe("useApplicationShutdownSupervision", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		globalThis.localStorage.clear();
		mockInvoke.mockImplementation(defaultInvoke);
	});

	it("uses only the backend-issued shutdown target capability", async () => {
		const { result, unmount } = renderHook(() =>
			useApplicationShutdownSupervision(),
		);
		await waitFor(() =>
			expect(result.current.state.shutdownTargets).toHaveLength(1),
		);

		await act(async () => {
			await result.current.retryShutdownTarget(
				result.current.state.shutdownTargets[0],
			);
		});

		expect(mockInvoke).toHaveBeenCalledWith("resolve_shutdown_target_action", {
			request: {
				action_id: "shutdown-action-1",
				shutdown_id: "plan-1",
				ordinal: "0",
				target_key: "target-key-1",
				origin_revision: "2",
				action: "retry_same_effect",
			},
		});
		unmount();
	});

	it("keeps outcome unknown distinct from no shutdown", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_application_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "get_application_shutdown":
					return Promise.resolve({
						type: "outcome_unknown",
						operation_id: "quit-unknown-1",
						intent: { type: "restart", code: 42 },
					});
				default:
					return Promise.resolve(undefined);
			}
		});
		const { result, unmount } = renderHook(() =>
			useApplicationShutdownSupervision(),
		);

		await waitFor(() =>
			expect(result.current.state.shutdownOutcomeUnknown).toEqual({
				operation_id: "quit-unknown-1",
				intent: { type: "restart", code: 42 },
			}),
		);
		expect(result.current.state.shutdown).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"get_shutdown_plan",
			expect.anything(),
		);
		unmount();
	});

	it("does not infer a retry capability from target state", async () => {
		const { result, unmount } = renderHook(() =>
			useApplicationShutdownSupervision(),
		);
		await waitFor(() =>
			expect(result.current.state.shutdownTargets).toHaveLength(1),
		);
		const target = {
			...result.current.state.shutdownTargets[0],
			actions: [],
		};

		await expect(result.current.retryShutdownTarget(target)).rejects.toThrow(
			"backend did not expose",
		);
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"resolve_shutdown_target_action",
			expect.anything(),
		);
		unmount();
	});

	it("retains the quit request id when its outcome is unknown", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_application_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "get_application_shutdown":
					return Promise.resolve({
						type: "current",
						plan: {
							shutdown_id: "failed-plan",
							phase: "failed",
							outcome: "aborted_before_activation",
							actions: ["retry_quit"],
						},
					});
				case "get_shutdown_plan":
					return Promise.resolve({ plan: {}, targets: [], next_cursor: null });
				case "request_application_quit":
					return Promise.resolve({ type: "outcome_unknown" });
				default:
					return Promise.resolve(undefined);
			}
		});
		const { result, unmount } = renderHook(() =>
			useApplicationShutdownSupervision(),
		);
		await waitFor(() =>
			expect(result.current.state.shutdown?.actions).toEqual(["retry_quit"]),
		);

		await act(async () => result.current.retryQuit());
		const snapshot = JSON.parse(
			globalThis.localStorage.getItem("releash:application-quit-attempt:v1") ??
				"null",
		) as { requestId: string };
		await act(async () => result.current.retryQuit());
		const calls = mockInvoke.mock.calls.filter(
			([command]) => command === "request_application_quit",
		);
		expect(calls).toHaveLength(2);
		expect(calls[0][1].request.request_id).toBe(snapshot.requestId);
		expect(calls[1][1].request.request_id).toBe(snapshot.requestId);
		unmount();
	});
});
