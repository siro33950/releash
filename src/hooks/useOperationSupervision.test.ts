import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useOperationSupervision } from "./useOperationSupervision";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@/hooks/useSessionStore", () => ({
	getAcceptedSendOperation: vi.fn().mockResolvedValue(null),
	redispatchPendingLifecycleAttempts: vi.fn().mockResolvedValue(undefined),
	redispatchPendingPermissionResponseAttempts: vi
		.fn()
		.mockResolvedValue(undefined),
	redispatchPendingSendAttempts: vi.fn().mockResolvedValue(undefined),
	redispatchPendingStopAttempts: vi.fn().mockResolvedValue(undefined),
}));

describe("useOperationSupervision", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		globalThis.localStorage.clear();
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_agent_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "list_pending_agent_recovery":
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
		});
	});

	it("uses only the backend-issued shutdown target capability", async () => {
		const { result, unmount } = renderHook(() =>
			useOperationSupervision("session-1"),
		);
		await waitFor(() =>
			expect(result.current.state.shutdownTargets).toHaveLength(1),
		);
		expect(result.current.state.shutdownTargets[0].observation).toEqual({
			type: "exit_coupled_outcome_unknown",
			shutdown_id: "plan-1",
		});

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

	it("keeps a first-writer shutdown outcome unknown distinct from no shutdown", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_agent_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "list_pending_agent_recovery":
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
			useOperationSupervision("session-1"),
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
		expect(mockInvoke).not.toHaveBeenCalledWith(
			"request_application_quit",
			expect.anything(),
		);
		unmount();
	});

	it("submits the backend-issued recovery action identity", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_agent_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "list_pending_agent_recovery":
					return Promise.resolve({
						entries: [
							{
								obligation_id: "permission-1",
								owner: "session-1",
								revision: "4",
								safe_label: "Retry the saved permission response",
								actions: ["retry_same_effect"],
								action_identities: [
									{
										action_id: "issued-action-1",
										action: "retry_same_effect",
										origin_revision: "4",
									},
								],
							},
						],
						next_cursor: null,
					});
				case "get_application_shutdown":
					return Promise.resolve({ type: "current", plan: null });
				default:
					return Promise.resolve(undefined);
			}
		});
		const { result, unmount } = renderHook(() =>
			useOperationSupervision("session-1"),
		);
		await waitFor(() => expect(result.current.state.recovery).toHaveLength(1));

		await act(async () => {
			await result.current.requestRecovery(
				result.current.state.recovery[0],
				"retry_same_effect",
			);
		});

		expect(mockInvoke).toHaveBeenCalledWith("resolve_pending_recovery_action", {
			request: {
				action_id: "issued-action-1",
				obligation_id: "permission-1",
				origin_revision: "4",
				action: "retry_same_effect",
			},
		});
		unmount();
	});

	it("recovers accepted attempts from the backend journal and scopes recovery at the query", async () => {
		mockInvoke.mockImplementation(
			(command: string, args?: Record<string, unknown>) => {
				switch (command) {
					case "list_pending_agent_attempts":
						return Promise.resolve({
							entries:
								args?.scopeId === "session-1"
									? [
											{
												kind: "stop",
												caller_request_id: "stop-request-1",
												operation_id: "stop-operation-1",
												resolution: "accepted",
												revision: "0",
											},
										]
									: [],
							next_cursor: null,
						});
					case "get_stop_operation":
						return Promise.resolve({
							type: "found",
							operation_id: "stop-operation-1",
						});
					case "list_pending_agent_recovery":
						return Promise.resolve({ entries: [], next_cursor: null });
					case "get_application_shutdown":
						return Promise.resolve({ type: "current", plan: null });
					default:
						return Promise.resolve(undefined);
				}
			},
		);

		const { result, unmount } = renderHook(() =>
			useOperationSupervision("session-1"),
		);
		await waitFor(() =>
			expect(result.current.state.operationReadbacks).toHaveLength(1),
		);

		expect(mockInvoke).toHaveBeenCalledWith("list_pending_agent_recovery", {
			limit: 32,
			partition: null,
			owner: "session-1",
			shutdownId: null,
			cursor: null,
		});
		expect(mockInvoke).toHaveBeenCalledWith("get_stop_operation", {
			operationId: "stop-operation-1",
		});
		expect(mockInvoke).toHaveBeenCalledWith("acknowledge_agent_attempt", {
			kind: "stop",
			callerRequestId: "stop-request-1",
		});
		unmount();
	});

	it("adopts a permission response identity and reads it back after renderer restart", async () => {
		let journalVisible = true;
		mockInvoke.mockImplementation(
			(command: string, args?: Record<string, unknown>) => {
				switch (command) {
					case "list_pending_agent_attempts":
						return Promise.resolve({
							entries:
								journalVisible && args?.scopeId === "session-1"
									? [
											{
												kind: "permission_response",
												caller_request_id: "permission-response-1",
												operation_id: "permission-response-1",
												resolution: "accepted",
											},
										]
									: [],
							next_cursor: null,
						});
					case "get_agent_permission_response_operation":
						return Promise.resolve({
							receipt: { operation_id: args?.operationId },
							latest_status: { type: "reconciliation_required" },
						});
					case "list_pending_agent_recovery":
						return Promise.resolve({ entries: [], next_cursor: null });
					case "get_application_shutdown":
						return Promise.resolve({ type: "current", plan: null });
					default:
						return Promise.resolve(undefined);
				}
			},
		);

		const first = renderHook(() => useOperationSupervision("session-1"));
		await waitFor(() =>
			expect(first.result.current.state.operationReadbacks).toHaveLength(1),
		);
		expect(mockInvoke).toHaveBeenCalledWith(
			"get_agent_permission_response_operation",
			{ operationId: "permission-response-1" },
		);
		expect(mockInvoke).toHaveBeenCalledWith("acknowledge_agent_attempt", {
			kind: "permission_response",
			callerRequestId: "permission-response-1",
		});
		first.unmount();

		journalVisible = false;
		mockInvoke.mockClear();
		const restarted = renderHook(() => useOperationSupervision("session-1"));
		await waitFor(() =>
			expect(restarted.result.current.state.operationReadbacks).toHaveLength(1),
		);
		expect(mockInvoke).toHaveBeenCalledWith(
			"get_agent_permission_response_operation",
			{ operationId: "permission-response-1" },
		);
		restarted.unmount();
	});

	it("adopts and acknowledges every accepted identity independently", async () => {
		const attempts = Array.from({ length: 33 }, (_, ordinal) => ({
			kind: "send",
			caller_request_id: `send-request-${ordinal}`,
			operation_id: `send-operation-${ordinal}`,
			resolution: "accepted",
		}));
		mockInvoke.mockImplementation(
			(command: string, args?: Record<string, unknown>) => {
				switch (command) {
					case "list_pending_agent_attempts":
						if (args?.scopeId !== "session-1") {
							return Promise.resolve({ entries: [], next_cursor: null });
						}
						if (args?.cursor === "accepted-page-2") {
							return Promise.resolve({
								entries: attempts.slice(32),
								next_cursor: null,
							});
						}
						return Promise.resolve({
							entries: attempts.slice(0, 32),
							next_cursor: "accepted-page-2",
						});
					case "get_agent_send_operation":
						return Promise.resolve({ operation_id: args?.operationId });
					case "list_pending_agent_recovery":
						return Promise.resolve({ entries: [], next_cursor: null });
					case "get_application_shutdown":
						return Promise.resolve({ type: "current", plan: null });
					default:
						return Promise.resolve(undefined);
				}
			},
		);

		const { result, unmount } = renderHook(() =>
			useOperationSupervision("session-1"),
		);
		await waitFor(() =>
			expect(result.current.state.operationReadbacks).toHaveLength(33),
		);
		const reads = mockInvoke.mock.calls.filter(
			([command]) => command === "get_agent_send_operation",
		);
		const acknowledgements = mockInvoke.mock.calls.filter(
			([command]) => command === "acknowledge_agent_attempt",
		);
		expect(new Set(reads.map(([, args]) => args.operationId)).size).toBe(33);
		expect(acknowledgements).toHaveLength(33);
		unmount();
	});

	it("does not acknowledge an accepted attempt whose identity readback failed", async () => {
		mockInvoke.mockImplementation(
			(command: string, args?: Record<string, unknown>) => {
				switch (command) {
					case "list_pending_agent_attempts":
						return Promise.resolve({
							entries:
								args?.scopeId === "session-1"
									? [
											{
												kind: "send",
												caller_request_id: "send-request-failed",
												operation_id: "send-operation-failed",
												resolution: "accepted",
											},
										]
									: [],
							next_cursor: null,
						});
					case "get_agent_send_operation":
						return Promise.reject(new Error("readback unavailable"));
					case "list_pending_agent_recovery":
						return Promise.resolve({ entries: [], next_cursor: null });
					case "get_application_shutdown":
						return Promise.resolve({ type: "current", plan: null });
					default:
						return Promise.resolve(undefined);
				}
			},
		);

		const { result, unmount } = renderHook(() =>
			useOperationSupervision("session-1"),
		);
		await waitFor(() =>
			expect(result.current.state.error).toContain("readback"),
		);
		expect(mockInvoke).not.toHaveBeenCalledWith("acknowledge_agent_attempt", {
			kind: "send",
			callerRequestId: "send-request-failed",
		});
		unmount();
	});

	it("does not infer a retry capability from target state", async () => {
		const { result, unmount } = renderHook(() =>
			useOperationSupervision("session-1"),
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

	it("retries quit only from the backend capability and retains its id on response loss", async () => {
		mockInvoke.mockImplementation((command: string) => {
			switch (command) {
				case "list_pending_agent_attempts":
					return Promise.resolve({ entries: [], next_cursor: null });
				case "list_pending_agent_recovery":
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
			useOperationSupervision("session-1"),
		);
		await waitFor(() =>
			expect(result.current.state.shutdown?.actions).toEqual(["retry_quit"]),
		);

		await act(async () => result.current.retryQuit());
		const first = JSON.parse(
			globalThis.localStorage.getItem("releash:application-quit-attempt:v1") ??
				"null",
		) as { requestId: string };
		expect(first.requestId).toMatch(/^quit-/);
		await act(async () => result.current.retryQuit());
		const calls = mockInvoke.mock.calls.filter(
			([command]) => command === "request_application_quit",
		);
		expect(calls).toHaveLength(2);
		expect(calls[0][1].request.request_id).toBe(first.requestId);
		expect(calls[1][1].request.request_id).toBe(first.requestId);
		unmount();
	});
});
