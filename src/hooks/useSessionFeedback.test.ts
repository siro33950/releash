import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useSessionFeedback } from "./useSessionFeedback";
import type { SessionFeedbackEntry } from "./useSessionStore";

const mockInvoke = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

const entry = {
	feedback_id: "feedback-1",
	attempt_id: "attempt-1",
	session_id: "session-a",
	operation: "send" as const,
	revision: "0",
	actions: ["dismiss" as const],
	action_identities: [
		{
			action: "dismiss" as const,
			action_id: "dismiss-action-0",
			origin_revision: "0",
		},
	],
	failure: {
		kind: "persist_failure",
		retryable: true,
		label: "send failed",
		detail: "safe detail",
		correlation_id: "correlation-1",
	},
};

describe("useSessionFeedback", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockInvoke.mockImplementation((command: string) => {
			if (command === "list_agent_session_feedback") {
				return Promise.resolve({ entries: [entry], next_cursor: null });
			}
			if (command === "retry_agent_session_feedback") {
				return Promise.resolve({ type: "resolved" });
			}
			return Promise.resolve(undefined);
		});
	});

	it("mirrors the bounded backend page and dismisses by identity and revision", async () => {
		const { result } = renderHook(() => useSessionFeedback("session-a"));
		await waitFor(() => expect(result.current.entries).toEqual([entry]));

		await act(async () => {
			await result.current.dismiss(entry);
		});

		expect(mockInvoke).toHaveBeenCalledWith("dismiss_agent_session_feedback", {
			sessionId: "session-a",
			feedbackId: "feedback-1",
			expectedRevision: "0",
			actionId: "dismiss-action-0",
		});
	});

	it("does not retain another session's page while identity changes", async () => {
		const { result, rerender } = renderHook(
			({ sessionId }) => useSessionFeedback(sessionId),
			{ initialProps: { sessionId: "session-a" as string | null } },
		);
		await waitFor(() => expect(result.current.entries).toHaveLength(1));
		rerender({ sessionId: null });
		expect(result.current.entries).toEqual([]);
	});

	it("retries the exact durable feedback identity and revision", async () => {
		const retryable: SessionFeedbackEntry = {
			...entry,
			actions: ["dismiss", "retry_resolution"],
			action_identities: [
				...entry.action_identities,
				{
					action: "retry_resolution",
					action_id: "retry-action-0",
					origin_revision: "0",
				},
			],
		};
		const { result } = renderHook(() => useSessionFeedback("session-a"));
		await waitFor(() => expect(result.current.entries).toEqual([entry]));

		await act(async () => {
			await result.current.retry(retryable);
		});

		expect(mockInvoke).toHaveBeenCalledWith("retry_agent_session_feedback", {
			sessionId: "session-a",
			feedbackId: "feedback-1",
			expectedRevision: "0",
			actionId: "retry-action-0",
		});
	});

	it("keeps a 33rd unresolved entry reachable through bounded paging", async () => {
		const firstPage = Array.from({ length: 32 }, (_, index) => ({
			...entry,
			feedback_id: `feedback-${index}`,
			attempt_id: `attempt-${index}`,
		}));
		const finalEntry = {
			...entry,
			feedback_id: "feedback-32",
			attempt_id: "attempt-32",
		};
		mockInvoke.mockImplementation(
			(command: string, args?: { cursor?: string | null }) => {
				if (command !== "list_agent_session_feedback") {
					return Promise.resolve(undefined);
				}
				return args?.cursor === "page-2"
					? Promise.resolve({ entries: [finalEntry], next_cursor: null })
					: Promise.resolve({ entries: firstPage, next_cursor: "page-2" });
			},
		);
		const { result } = renderHook(() => useSessionFeedback("session-a"));
		await waitFor(() => expect(result.current.entries).toHaveLength(32));
		expect(result.current.hasMore).toBe(true);

		await act(async () => {
			await result.current.loadNextPage();
		});

		expect(result.current.entries).toHaveLength(33);
		expect(result.current.entries[32]).toEqual(finalEntry);
		expect(result.current.hasMore).toBe(false);
	});

	it("preserves the last backend snapshot when query or control fails", async () => {
		const { result } = renderHook(() => useSessionFeedback("session-a"));
		await waitFor(() => expect(result.current.entries).toEqual([entry]));

		mockInvoke.mockRejectedValueOnce(new Error("query unavailable"));
		await act(async () => {
			await expect(result.current.refresh()).rejects.toThrow(
				"query unavailable",
			);
		});
		expect(result.current.entries).toEqual([entry]);

		mockInvoke.mockRejectedValueOnce(new Error("control unavailable"));
		await act(async () => {
			await expect(result.current.dismiss(entry)).rejects.toThrow(
				"control unavailable",
			);
		});
		expect(result.current.entries).toEqual([entry]);
	});
});
