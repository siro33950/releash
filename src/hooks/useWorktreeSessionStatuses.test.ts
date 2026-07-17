import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { SessionNotice, SessionStatus } from "@/types/session";
import { useWorktreeSessionStatuses } from "./useWorktreeSessionStatuses";

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

describe("useWorktreeSessionStatuses", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
	});

	it("returns empty Map when worktreePath is null", () => {
		const { result } = renderHook(() => useWorktreeSessionStatuses(null));
		expect(result.current.size).toBe(0);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("filters initial statuses by worktree_id", async () => {
		const a = makeStatus({ chat_session_id: "a", worktree_id: "/tmp/wt" });
		const b = makeStatus({ chat_session_id: "b", worktree_id: "/tmp/wt" });
		const c = makeStatus({ chat_session_id: "c", worktree_id: "/tmp/other" });
		mockInvoke.mockResolvedValue([a, b, c]);

		const { result } = renderHook(() => useWorktreeSessionStatuses("/tmp/wt"));

		await waitFor(() => {
			expect(result.current.size).toBe(2);
		});
		expect(result.current.get("a")).toEqual(a);
		expect(result.current.get("b")).toEqual(b);
		expect(result.current.get("c")).toBeUndefined();
	});

	it("merges matching session-status-changed events", async () => {
		mockInvoke.mockResolvedValue([
			makeStatus({ chat_session_id: "a", agent_state: "done" }),
		]);

		type Cb = (event: { payload: SessionStatus }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "session-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeSessionStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(result.current.get("a")?.agent_state).toBe("done");
		});

		await act(async () => {
			cb?.({
				payload: makeStatus({ chat_session_id: "a", agent_state: "running" }),
			});
		});

		expect(result.current.get("a")?.agent_state).toBe("running");
	});

	it("ignores events for other worktrees", async () => {
		mockInvoke.mockResolvedValue([
			makeStatus({ chat_session_id: "a", agent_state: "done" }),
		]);

		type Cb = (event: { payload: SessionStatus }) => void;
		let cb: Cb | null = null;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "session-status-changed") cb = fn;
			return Promise.resolve(vi.fn());
		});

		const { result } = renderHook(() => useWorktreeSessionStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(result.current.size).toBe(1);
		});

		await act(async () => {
			cb?.({
				payload: makeStatus({
					chat_session_id: "z",
					worktree_id: "/tmp/other",
					agent_state: "error",
				}),
			});
		});

		expect(result.current.size).toBe(1);
		expect(result.current.get("z")).toBeUndefined();
	});

	it("merges session-scoped notice events into the backend status snapshot", async () => {
		mockInvoke.mockResolvedValue([makeStatus()]);
		type EventPayload = SessionStatus | SessionNotice;
		type Cb = (event: { payload: EventPayload }) => void;
		const callbacks: Record<string, Cb> = {};
		mockListen.mockImplementation((event: string, fn: Cb) => {
			callbacks[event] = fn;
			return Promise.resolve(vi.fn());
		});
		const { result } = renderHook(() => useWorktreeSessionStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(result.current.has("session-1")).toBe(true);
		});
		const notice: SessionNotice = {
			sessionId: "session-1",
			kind: "persist_failure",
			message: "Unable to save the session.",
			createdAt: 2_000,
		};

		await act(async () => {
			callbacks["agent-session-notice"]?.({ payload: notice });
		});

		expect(result.current.get("session-1")?.notice).toEqual(notice);
	});

	it("keeps a notice pushed while the initial status snapshot is pending", async () => {
		type EventPayload = SessionStatus | SessionNotice;
		type Cb = (event: { payload: EventPayload }) => void;
		const callbacks: Record<string, Cb> = {};
		mockListen.mockImplementation((event: string, fn: Cb) => {
			callbacks[event] = fn;
			return Promise.resolve(vi.fn());
		});
		let resolveInitial: ((statuses: SessionStatus[]) => void) | undefined;
		mockInvoke.mockImplementation(
			() =>
				new Promise<SessionStatus[]>((resolve) => {
					resolveInitial = resolve;
				}),
		);
		const { result } = renderHook(() => useWorktreeSessionStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(callbacks["agent-session-notice"]).toBeDefined();
		});
		const notice: SessionNotice = {
			sessionId: "session-1",
			kind: "persist_failure",
			message: "Unable to save the session.",
			createdAt: 2_000,
		};

		await act(async () => {
			callbacks["session-status-changed"]?.({
				payload: makeStatus({ notice: null, last_activity_at: 1_000 }),
			});
			callbacks["agent-session-notice"]?.({ payload: notice });
			resolveInitial?.([makeStatus({ notice: null, last_activity_at: 1_000 })]);
		});

		await waitFor(() => {
			expect(result.current.get("session-1")?.notice).toEqual(notice);
		});
	});

	it("keeps a newer snapshot notice over an older pending push", async () => {
		type EventPayload = SessionStatus | SessionNotice;
		type Cb = (event: { payload: EventPayload }) => void;
		const callbacks: Record<string, Cb> = {};
		mockListen.mockImplementation((event: string, fn: Cb) => {
			callbacks[event] = fn;
			return Promise.resolve(vi.fn());
		});
		let resolveInitial: ((statuses: SessionStatus[]) => void) | undefined;
		mockInvoke.mockImplementation(
			() =>
				new Promise<SessionStatus[]>((resolve) => {
					resolveInitial = resolve;
				}),
		);
		const { result } = renderHook(() => useWorktreeSessionStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(callbacks["agent-session-notice"]).toBeDefined();
		});
		const pendingNotice: SessionNotice = {
			sessionId: "session-1",
			kind: "persist_failure",
			message: "Older pending notice",
			createdAt: 2_000,
		};
		const snapshotNotice: SessionNotice = {
			sessionId: "session-1",
			kind: "event_log_recovered",
			message: "Newer snapshot notice",
			createdAt: 3_000,
		};

		await act(async () => {
			callbacks["agent-session-notice"]?.({ payload: pendingNotice });
			resolveInitial?.([makeStatus({ notice: snapshotNotice })]);
		});

		await waitFor(() => {
			expect(result.current.get("session-1")?.notice).toEqual(snapshotNotice);
		});
	});

	it("applies a backend status push that clears a persist notice", async () => {
		const notice: SessionNotice = {
			sessionId: "session-1",
			kind: "persist_failure",
			message: "Unable to save the session.",
			createdAt: 2_000,
		};
		mockInvoke.mockResolvedValue([makeStatus({ notice })]);
		type Cb = (event: { payload: SessionStatus }) => void;
		let statusChanged: Cb | undefined;
		mockListen.mockImplementation((event: string, fn: Cb) => {
			if (event === "session-status-changed") statusChanged = fn;
			return Promise.resolve(vi.fn());
		});
		const { result } = renderHook(() => useWorktreeSessionStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(result.current.get("session-1")?.notice).toEqual(notice);
		});

		await act(async () => {
			statusChanged?.({ payload: makeStatus({ notice: null }) });
		});

		expect(result.current.get("session-1")?.notice).toBeNull();
	});

	it("returns empty Map when invoke fails", async () => {
		mockInvoke.mockRejectedValue(new Error("boom"));
		const { result } = renderHook(() => useWorktreeSessionStatuses("/tmp/wt"));
		await waitFor(() => {
			expect(result.current.size).toBe(0);
		});
	});
});
