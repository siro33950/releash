import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionStatus } from "@/types/session";
import { useSessionStatus } from "./useSessionStatus";

const mockInvoke = vi.fn();
const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: (...args: unknown[]) => mockInvoke(...args),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

const makeStatus = (overrides: Partial<SessionStatus> = {}): SessionStatus => ({
	chat_session_id: "session-1",
	worktree_id: "/tmp/wt",
	worktree_path: "/tmp/wt",
	pty_id: null,
	agent_state: "running",
	turn_phase: "streaming",
	session_state: "active",
	pending_permission: false,
	last_activity_at: 1_000,
	...overrides,
});

describe("useSessionStatus", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("returns null when chatSessionId is null", () => {
		const { result } = renderHook(() => useSessionStatus(null));
		expect(result.current).toBeNull();
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("fetches initial status via get_session_status", async () => {
		const status = makeStatus();
		mockInvoke.mockResolvedValue(status);

		const { result } = renderHook(() => useSessionStatus("session-1"));

		await waitFor(() => {
			expect(result.current).toEqual(status);
		});
		expect(mockInvoke).toHaveBeenCalledWith("get_session_status", {
			chatSessionId: "session-1",
		});
	});

	it("updates status when matching session-status-changed fires", async () => {
		mockInvoke.mockResolvedValue(makeStatus({ agent_state: "done" }));

		type Cb = (event: { payload: SessionStatus }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "session-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useSessionStatus("session-1"));
		await waitFor(() => {
			expect(result.current?.agent_state).toBe("done");
		});

		await act(async () => {
			cb?.({ payload: makeStatus({ agent_state: "running" }) });
		});

		expect(result.current?.agent_state).toBe("running");
	});

	it("ignores events for other sessions", async () => {
		mockInvoke.mockResolvedValue(makeStatus({ agent_state: "done" }));

		type Cb = (event: { payload: SessionStatus }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "session-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useSessionStatus("session-1"));
		await waitFor(() => {
			expect(result.current?.agent_state).toBe("done");
		});

		await act(async () => {
			cb?.({
				payload: makeStatus({
					chat_session_id: "session-other",
					agent_state: "error",
				}),
			});
		});

		expect(result.current?.agent_state).toBe("done");
	});

	it("returns null when invoke fails", async () => {
		mockInvoke.mockRejectedValue(new Error("not found"));
		const { result } = renderHook(() => useSessionStatus("session-1"));
		await waitFor(() => {
			expect(result.current).toBeNull();
		});
	});
});
