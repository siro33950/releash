import { invoke } from "@tauri-apps/api/core";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
	createWorkspaceSession,
	getSession,
	getSessionPage,
	listAcceptedPermissionResponseOperations,
	planAgentChatEviction,
	redispatchPendingPermissionResponseAttempts,
	requestAgentStop,
	respondAgentPermission,
	resumeAgentQueue,
	sendAgentMessage,
	sendWorkflowApprovalChatMessage,
} from "./useSessionStore";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));

describe("session paging", () => {
	beforeEach(() => {
		vi.mocked(invoke).mockReset();
	});

	it("getSession uses messages and initial page metadata returned by get_session", async () => {
		vi.mocked(invoke).mockResolvedValueOnce({
			id: "s1",
			worktree_path: "/repo",
			messages: [
				{
					id: "m1",
					role: "human",
					content: "hello",
					thinking: null,
					activities: null,
					parts: null,
					streaming_final_seq: "0",
					timestamp_ms: "1001000",
					mentions: null,
				},
			],
			state: "error",
			error_reason: "app server stopped",
			created_at_ms: "1000000",
			updated_at_ms: "1000000",
			agent_session_id: null,
			context_carry: null,
			permission_mode: "edit",
			plan_mode: false,
			permission_profile_id: null,
			backend_id: null,
			selected_model: "claude:sonnet",
			available_models: [],
			can_change_backend: true,
			pending_queue: [],
			pending_queue_count: "0",
			queue_paused: false,
			workflow_node_session: false,
			workflow_node_context: null,
			session_revision: "0",
			active_turn_id: null,
			turn_phase: "idle",
			initial_page: {
				next_cursor: "1",
				has_more: true,
				total_count: "10",
			},
			pending_permission_request: {
				id: "perm-1",
				tool_use_id: "toolu-1",
				tool_name: "Bash",
				kind: "tool_approval",
				input: { command: "echo hi" },
				plan: null,
				allowed_prompts: [],
				questions: [],
				title: null,
				display_name: null,
				description: null,
				decision_reason: null,
			},
			pending_permission_state_revision: "4",
			latest_token_usage: {
				input_tokens: "1",
				output_tokens: "2",
				total_tokens: null,
				context_window_tokens: null,
			},
			last_turn_interruption: {
				message_id: "agent-1",
				reason: "session_closed",
			},
		});

		const response = await getSession("s1");

		expect(invoke).toHaveBeenCalledTimes(1);
		expect(invoke).toHaveBeenCalledWith("get_session", {
			sessionId: "s1",
			attemptId: expect.stringMatching(/^load-/),
		});
		expect(response?.session.messages).toEqual([
			{
				id: "m1",
				role: "human",
				parts: [{ type: "text", content: "hello" }],
				timestamp: 1001,
				mentions: undefined,
			},
		]);
		expect(response?.session.errorReason).toBe("app server stopped");
		expect(response?.initialPage).toEqual({
			nextCursor: "1",
			hasMore: true,
			totalCount: 10,
		});
		expect(response?.pendingPermissionRequest).toEqual({
			id: "perm-1",
			toolName: "Bash",
			kind: "tool_approval",
			input: { command: "echo hi" },
			toolUseId: "toolu-1",
		});
		expect(response?.pendingPermissionStateRevision).toBe("4");
		expect(response?.latestTokenUsage).toEqual({
			inputTokens: 1,
			outputTokens: 2,
		});
		expect(response?.session.lastTurnInterruption).toEqual({
			messageId: "agent-1",
			reason: "session_closed",
		});
	});

	it("getSession keeps one caller attempt across failure and rotates after success", async () => {
		vi.mocked(invoke)
			.mockRejectedValueOnce(new Error("reply lost"))
			.mockResolvedValueOnce(null)
			.mockResolvedValueOnce(null);

		await expect(getSession("load-retry-session")).rejects.toThrow(
			"reply lost",
		);
		const firstAttempt = (
			vi.mocked(invoke).mock.calls[0]?.[1] as { attemptId?: string } | undefined
		)?.attemptId;
		await expect(getSession("load-retry-session")).resolves.toBeNull();
		const replayAttempt = (
			vi.mocked(invoke).mock.calls[1]?.[1] as { attemptId?: string } | undefined
		)?.attemptId;
		expect(replayAttempt).toBe(firstAttempt);

		await expect(getSession("load-retry-session")).resolves.toBeNull();
		const nextAttempt = (
			vi.mocked(invoke).mock.calls[2]?.[1] as { attemptId?: string } | undefined
		)?.attemptId;
		expect(nextAttempt).not.toBe(firstAttempt);
	});

	it("createWorkspaceSession forwards the stable idempotency key", async () => {
		vi.mocked(invoke).mockResolvedValueOnce("request-uuid");

		const sessionId = await createWorkspaceSession(
			"request-uuid",
			"/repo",
			"ask",
			"claude",
			"sonnet",
		);

		expect(invoke).toHaveBeenCalledWith("create_workspace_session", {
			requestId: "request-uuid",
			worktreePath: "/repo",
			permissionMode: "ask",
			backendId: "claude",
			modelId: "sonnet",
		});
		expect(sessionId).toBe("request-uuid");
	});

	it("getSessionPage forwards cursor and limit", async () => {
		vi.mocked(invoke).mockResolvedValueOnce({
			messages: [
				{
					id: "m2",
					role: "human",
					content: "older",
					thinking: null,
					activities: null,
					parts: null,
					streaming_final_seq: "0",
					timestamp_ms: "1000000",
					mentions: null,
				},
			],
			message_metadata: [
				{ message_id: "m2", token_meta: { input: 1 }, run_meta: null },
			],
			next_cursor: null,
			has_more: false,
			total_count: "1",
			latest_token_usage: null,
		});

		const page = await getSessionPage("s1", "7", 25);

		expect(invoke).toHaveBeenCalledWith("get_session_page", {
			sessionId: "s1",
			cursor: "7",
			limit: 25,
		});
		expect(page).toEqual({
			messages: [
				{
					id: "m2",
					role: "human",
					parts: [{ type: "text", content: "older" }],
					timestamp: 1000,
					mentions: undefined,
				},
			],
			messageMetadata: [{ messageId: "m2", tokenMeta: { input: 1 } }],
			nextCursor: null,
			hasMore: false,
			totalCount: 1,
			latestTokenUsage: null,
		});
	});

	it("planAgentChatEviction forwards request and returns the plan unchanged", async () => {
		const request = {
			active: {
				sessionId: "s1",
				messageCount: 250,
				oldestVisibleIndex: 50,
				loadedPages: [
					{ requestCursor: null, count: 50 },
					{ requestCursor: "201", count: 50 },
				],
				turnPhase: "idle" as const,
			},
			sessions: [
				{
					sessionId: "s2",
					messageCount: 50,
					evictionRank: 1,
					protected: false,
					loading: false,
				},
			],
		};
		const plan = {
			active: {
				sessionId: "s1",
				direction: "older" as const,
				count: 50,
				nextCursor: "201",
				hasMore: true,
				loadedPages: [{ requestCursor: null, count: 50 }],
			},
			evictSessionIds: ["s2"],
		};
		vi.mocked(invoke).mockResolvedValueOnce(plan);

		const response = await planAgentChatEviction(request);

		expect(invoke).toHaveBeenCalledWith("plan_agent_chat_eviction", {
			request,
		});
		expect(response).toBe(plan);
	});

	it("sendWorkflowApprovalChatMessage uses a durable operation identity", async () => {
		vi.mocked(invoke).mockResolvedValueOnce({
			type: "accepted",
			operation: {
				receipt: {
					operation_id: "send-workflow-1",
					session_id: "s1",
					input_ref: "input-workflow-1",
					disposition: { type: "started_turn", turn_id: "1" },
				},
				latest_status: { type: "running", turn_id: "1" },
			},
		});

		await sendWorkflowApprovalChatMessage("run-1", "approve", "edit", false);

		expect(invoke).toHaveBeenCalledWith("send_workflow_approval_chat_message", {
			operationId: expect.stringMatching(/^send-/),
			executionId: "run-1",
			content: "approve",
			permissionMode: "edit",
			planMode: false,
			images: undefined,
			mentions: undefined,
		});
	});

	it("sendWorkflowApprovalChatMessage reads back the same operation after response loss", async () => {
		vi.mocked(invoke)
			.mockRejectedValueOnce(new Error("response lost"))
			.mockResolvedValueOnce({
				receipt: {
					operation_id: "send-workflow-readback",
					session_id: "s-workflow-readback",
					input_ref: "input-workflow-readback",
					disposition: { type: "started_turn", turn_id: "9" },
				},
				latest_status: { type: "provider_start_reserved" },
			});

		const result = await sendWorkflowApprovalChatMessage(
			"run-response-loss",
			"approve after loss",
			"edit",
			false,
		);

		const firstArgs = vi.mocked(invoke).mock.calls[0]?.[1] as {
			operationId?: string;
		};
		expect(firstArgs.operationId).toMatch(/^send-/);
		expect(invoke).toHaveBeenNthCalledWith(2, "get_agent_send_operation", {
			operationId: firstArgs.operationId,
		});
		expect(result.type).toBe("accepted");
	});

	it("sendAgentMessage omits client timing args", async () => {
		vi.mocked(invoke).mockResolvedValueOnce({
			type: "accepted",
			operation: {
				receipt: {
					operation_id: "send-1",
					session_id: "s1",
					input_ref: "input-1",
					disposition: { type: "started_turn", turn_id: "1" },
				},
				latest_status: { type: "running", turn_id: "1" },
			},
		});

		await sendAgentMessage(
			"s1",
			"/repo",
			"hello",
			"edit",
			false,
			"claude",
			[],
			[],
			undefined,
			"claude-sonnet-4-6",
		);

		expect(invoke).toHaveBeenCalledWith("send_agent_message", {
			operationId: expect.stringMatching(/^send-/),
			chatSessionId: "s1",
			worktreePath: "/repo",
			content: "hello",
			permissionMode: "edit",
			planMode: false,
			backendId: "claude",
			modelId: "claude-sonnet-4-6",
			images: undefined,
			mentions: undefined,
		});
	});

	it("sendAgentMessage reads back the same operation after response loss", async () => {
		vi.mocked(invoke)
			.mockRejectedValueOnce(new Error("response lost"))
			.mockResolvedValueOnce({
				receipt: {
					operation_id: "send-readback",
					session_id: "s-readback",
					input_ref: "input-readback",
					disposition: { type: "started_turn", turn_id: "7" },
				},
				latest_status: { type: "provider_start_reserved" },
			});

		const result = await sendAgentMessage(
			"s-readback",
			"/repo",
			"response-loss-input",
			"edit",
			false,
		);

		const firstArgs = vi.mocked(invoke).mock.calls[0]?.[1] as {
			operationId?: string;
		};
		const firstOperationId = firstArgs.operationId;
		expect(firstOperationId).toMatch(/^send-/);
		expect(invoke).toHaveBeenNthCalledWith(2, "get_agent_send_operation", {
			operationId: firstOperationId,
		});
		expect(result.type).toBe("accepted");
	});

	it("send query OutcomeUnknown stays unresolved and retries the same identity", async () => {
		let pendingOperationId = "";
		vi.mocked(invoke)
			.mockImplementationOnce((_command, args) => {
				pendingOperationId = (args as { operationId: string }).operationId;
				return Promise.resolve({
					type: "outcome_unknown",
					operation_id: pendingOperationId,
				});
			})
			.mockImplementationOnce(() =>
				Promise.reject({
					type: "outcome_unknown",
					operation_id: pendingOperationId,
				}),
			);

		await expect(
			sendAgentMessage(
				"s-query-unknown",
				"/repo",
				"query unknown input",
				"edit",
				false,
			),
		).rejects.toThrow(/Send acceptance is unknown; retry operation send-/);
		expect(pendingOperationId).toMatch(/^send-/);
		expect(invoke).toHaveBeenNthCalledWith(2, "get_agent_send_operation", {
			operationId: pendingOperationId,
		});

		vi.mocked(invoke).mockImplementationOnce((_command, args) => {
			const operationId = (args as { operationId: string }).operationId;
			return Promise.resolve({
				type: "accepted",
				operation: {
					receipt: {
						operation_id: operationId,
						session_id: "s-query-unknown",
						input_ref: "input-query-unknown",
						disposition: { type: "started_turn", turn_id: "1" },
					},
					latest_status: { type: "provider_start_reserved" },
				},
			});
		});
		const retry = await sendAgentMessage(
			"s-query-unknown",
			"/repo",
			"query unknown input",
			"edit",
			false,
		);
		const retryArgs = vi.mocked(invoke).mock.calls[2]?.[1] as {
			operationId: string;
		};
		expect(retryArgs.operationId).toBe(pendingOperationId);
		expect(retry.type).toBe("accepted");
	});

	it("respondAgentPermission reads back the same durable identity after response loss", async () => {
		vi.mocked(invoke)
			.mockRejectedValueOnce(new Error("response lost"))
			.mockImplementationOnce((_command, args) =>
				Promise.resolve({
					receipt: {
						operation_id: (args as { operationId: string }).operationId,
						session_id: "permission-session-loss",
						request_id: "permission-request-loss",
						input_ref: "permission-input-loss",
					},
					latest_status: { type: "reconciliation_required" },
				}),
			);

		const updatedInput = { answers: { Question: "Answer" } };
		const result = await respondAgentPermission(
			"permission-session-loss",
			"permission-request-loss",
			true,
			updatedInput,
		);
		const firstArgs = vi.mocked(invoke).mock.calls[0]?.[1] as {
			operationId: string;
			updatedInput: string;
		};
		expect(firstArgs.operationId).toMatch(/^permission-response-/);
		expect(firstArgs.updatedInput).toBe(JSON.stringify(updatedInput));
		expect(invoke).toHaveBeenNthCalledWith(
			2,
			"get_agent_permission_response_operation",
			{ operationId: firstArgs.operationId },
		);
		expect(result.type).toBe("accepted");
	});

	it("respondAgentPermission keeps one identity across a rejected retry and pending redispatch", async () => {
		vi.mocked(invoke).mockResolvedValueOnce({
			type: "rejected_before_commit",
			failure: { kind: "storage_unavailable" },
		});
		await expect(
			respondAgentPermission(
				"permission-session-retry",
				"permission-request-retry",
				false,
			),
		).rejects.toThrow("rejected before");
		const firstCall = vi.mocked(invoke).mock.calls[0];
		if (!firstCall)
			throw new Error("permission response invoke was not called");
		const operationId = (firstCall[1] as { operationId: string }).operationId;

		vi.mocked(invoke).mockResolvedValueOnce({
			type: "accepted",
			operation: {
				receipt: {
					operation_id: operationId,
					session_id: "permission-session-retry",
					request_id: "permission-request-retry",
					input_ref: "permission-input-retry",
				},
				latest_status: { type: "completed", decision: "denied" },
			},
		});
		await redispatchPendingPermissionResponseAttempts(new Set([operationId]));
		expect(invoke).toHaveBeenNthCalledWith(
			2,
			"respond_agent_permission",
			expect.objectContaining({
				operationId,
				chatSessionId: "permission-session-retry",
				requestId: "permission-request-retry",
				behavior: "deny",
				message: "User denied",
			}),
		);
	});

	it("requestAgentStop keeps one identity while accepted and clears it after terminal readback", async () => {
		vi.mocked(invoke)
			.mockResolvedValueOnce({
				type: "accepted",
				receipt: {
					operation_id: "backend-stop-operation",
					session_id: "stop-session-accepted",
					turn_id: "7",
					accepted_revision: "9",
				},
				state: { type: "accepted" },
			})
			.mockResolvedValueOnce({
				type: "accepted",
				receipt: {
					operation_id: "backend-stop-operation",
					session_id: "stop-session-accepted",
					turn_id: "7",
					accepted_revision: "9",
				},
				state: { type: "completed", resolution: "superseded" },
			});

		await requestAgentStop("stop-session-accepted", "7", "9");

		const pending = JSON.parse(
			globalThis.localStorage.getItem("releash.agent-stop-attempts.v1") ?? "[]",
		) as [string, string][];
		const acceptedAttempt = pending.find(
			([key]) => key === "stop-session-accepted:7:9",
		);
		expect(acceptedAttempt?.[1]).toMatch(/^stop-/);

		await requestAgentStop("stop-session-accepted", "7", "9");

		const firstCall = vi.mocked(invoke).mock.calls[0];
		const secondCall = vi.mocked(invoke).mock.calls[1];
		if (!firstCall || !secondCall) {
			throw new Error("both stop invokes must be present");
		}
		const firstRequestId = (
			firstCall[1] as {
				request: { request_id: string };
			}
		).request.request_id;
		const secondRequestId = (
			secondCall[1] as {
				request: { request_id: string };
			}
		).request.request_id;
		expect(secondRequestId).toBe(firstRequestId);
		const terminal = JSON.parse(
			globalThis.localStorage.getItem("releash.agent-stop-attempts.v1") ?? "[]",
		) as [string, string][];
		expect(terminal).not.toContainEqual([
			"stop-session-accepted:7:9",
			firstRequestId,
		]);
	});

	it("requestAgentStop clears its renderer attempt after rejection before commit", async () => {
		vi.mocked(invoke).mockResolvedValueOnce({
			type: "rejected_before_commit",
			failure: { kind: "storage_unavailable" },
		});

		await expect(
			requestAgentStop("stop-session-rejected", "11", "12"),
		).rejects.toThrow("rejected before commit");

		const stored = JSON.parse(
			globalThis.localStorage.getItem("releash.agent-stop-attempts.v1") ?? "[]",
		) as [string, string][];
		expect(stored).not.toContainEqual([
			"stop-session-rejected:11:12",
			expect.any(String),
		]);
		expect(invoke).toHaveBeenCalledTimes(1);
	});

	it("requestAgentStop resolves response loss through the public caller-key lookup and clears the attempt", async () => {
		vi.mocked(invoke)
			.mockRejectedValueOnce(new Error("response lost"))
			.mockResolvedValueOnce([
				{
					operation_id: "backend-stop-readback",
					session_id: "stop-session-readback",
					turn_id: "8",
					accepted_revision: "10",
				},
				{ type: "completed", resolution: "succeeded" },
			]);

		await requestAgentStop("stop-session-readback", "8", "10");

		const firstCall = vi.mocked(invoke).mock.calls[0];
		if (!firstCall) throw new Error("stop invoke was not called");
		const requestId = (firstCall[1] as { request: { request_id: string } })
			.request.request_id;
		expect(requestId).toMatch(/^stop-/);
		expect(invoke).toHaveBeenNthCalledWith(2, "get_stop_operation", {
			operationId: requestId,
		});
		const stored = JSON.parse(
			globalThis.localStorage.getItem("releash.agent-stop-attempts.v1") ?? "[]",
		) as [string, string][];
		expect(stored).not.toContainEqual([
			"stop-session-readback:8:10",
			requestId,
		]);
	});

	it("bounds accepted operation mirrors to the supervision identity limit", async () => {
		vi.mocked(invoke).mockImplementation((_command, args) => {
			const input = args as {
				operationId: string;
				chatSessionId: string;
			};
			return Promise.resolve({
				type: "accepted",
				operation: {
					receipt: {
						operation_id: input.operationId,
						session_id: input.chatSessionId,
						input_ref: `input-${input.chatSessionId}`,
						disposition: { type: "started_turn", turn_id: "1" },
					},
					latest_status: { type: "running", turn_id: "1" },
				},
			});
		});

		for (let index = 0; index < 513; index += 1) {
			await sendAgentMessage(
				`bounded-session-${index}`,
				"/repo",
				`message-${index}`,
				"edit",
				false,
			);
		}

		const stored = JSON.parse(
			globalThis.localStorage.getItem("releash.accepted-send-operations.v1") ??
				"[]",
		) as [string, string][];
		expect(stored).toHaveLength(512);
		expect(stored.some(([key]) => key === "bounded-session-0")).toBe(false);
		expect(stored.some(([key]) => key === "bounded-session-512")).toBe(true);
	});

	it("preserves every unresolved caller attempt beyond the accepted mirror limit", async () => {
		vi.mocked(invoke).mockResolvedValue({
			type: "rejected_before_commit",
			failure: { kind: "storage_unavailable" },
		});

		const results = await Promise.allSettled(
			Array.from({ length: 513 }, (_, index) =>
				respondAgentPermission(
					`pending-over-capacity-session-${index}`,
					`pending-over-capacity-request-${index}`,
					false,
				),
			),
		);
		expect(results.every((result) => result.status === "rejected")).toBe(true);

		const stored = JSON.parse(
			globalThis.localStorage.getItem(
				"releash.agent-permission-response-attempts.v1",
			) ?? "[]",
		) as [string, string][];
		const pending = stored.filter(([key]) => {
			const snapshot = JSON.parse(key) as { sessionId?: string };
			return snapshot.sessionId?.startsWith("pending-over-capacity-session-");
		});
		expect(pending).toHaveLength(513);
		const firstIdentity = pending.find(([key]) => {
			const snapshot = JSON.parse(key) as { sessionId?: string };
			return snapshot.sessionId === "pending-over-capacity-session-0";
		})?.[1];
		expect(firstIdentity).toMatch(/^permission-response-/);

		await expect(
			respondAgentPermission(
				"pending-over-capacity-session-0",
				"pending-over-capacity-request-0",
				false,
			),
		).rejects.toThrow("rejected before");
		const invokeCalls = vi.mocked(invoke).mock.calls;
		const retry = invokeCalls[invokeCalls.length - 1]?.[1] as
			| { operationId?: string }
			| undefined;
		expect(retry?.operationId).toBe(firstIdentity);
	});

	it("resumeAgentQueue invokes the explicit queue resume command", async () => {
		await resumeAgentQueue("s1");

		expect(invoke).toHaveBeenCalledWith("resume_agent_queue", {
			chatSessionId: "s1",
		});
	});

	it("reads back an accepted permission response that later needs reconciliation", async () => {
		vi.mocked(invoke).mockImplementationOnce((_command, args) => {
			const input = args as { operationId: string };
			return Promise.resolve({
				type: "accepted",
				operation: {
					receipt: {
						operation_id: input.operationId,
						session_id: "permission-readback-session",
						request_id: "permission-readback-request",
						input_ref: "permission-readback-input",
					},
					latest_status: { type: "awaiting_provider_response" },
				},
			});
		});
		const accepted = await respondAgentPermission(
			"permission-readback-session",
			"permission-readback-request",
			true,
		);
		const operationId = accepted.operation.receipt.operation_id;

		vi.mocked(invoke).mockResolvedValueOnce({
			receipt: {
				operation_id: operationId,
				session_id: "permission-readback-session",
				request_id: "permission-readback-request",
				input_ref: "permission-readback-input",
			},
			latest_status: {
				type: "reconciliation_required",
				failure: { kind: "storage_unavailable" },
			},
		});

		const operations = await listAcceptedPermissionResponseOperations(
			"permission-readback-session",
		);

		expect(invoke).toHaveBeenLastCalledWith(
			"get_agent_permission_response_operation",
			{ operationId },
		);
		expect(operations).toHaveLength(1);
		expect(operations[0]?.latest_status.type).toBe("reconciliation_required");
	});

	it("drops the mirrored identity once the permission decision is settled", async () => {
		vi.mocked(invoke).mockImplementationOnce((_command, args) => {
			const input = args as { operationId: string };
			return Promise.resolve({
				type: "accepted",
				operation: {
					receipt: {
						operation_id: input.operationId,
						session_id: "permission-settled-session",
						request_id: "permission-settled-request",
						input_ref: "permission-settled-input",
					},
					latest_status: { type: "awaiting_provider_response" },
				},
			});
		});
		await respondAgentPermission(
			"permission-settled-session",
			"permission-settled-request",
			true,
		);

		vi.mocked(invoke).mockResolvedValueOnce({
			receipt: {
				operation_id: "permission-settled-operation",
				session_id: "permission-settled-session",
				request_id: "permission-settled-request",
				input_ref: "permission-settled-input",
			},
			latest_status: { type: "completed", decision: "allowed" },
		});
		expect(
			await listAcceptedPermissionResponseOperations(
				"permission-settled-session",
			),
		).toHaveLength(0);

		const readbackCalls = vi.mocked(invoke).mock.calls.length;
		expect(
			await listAcceptedPermissionResponseOperations(
				"permission-settled-session",
			),
		).toHaveLength(0);
		expect(vi.mocked(invoke).mock.calls).toHaveLength(readbackCalls);
	});
});
