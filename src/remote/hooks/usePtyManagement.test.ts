import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type { WsMessage } from "@/types/protocol";
import { usePtyManagement } from "./usePtyManagement";

describe("usePtyManagement", () => {
	let subscribers: ((msg: WsMessage) => void)[];
	let sentMessages: WsMessage[];
	let subscribe: (cb: (msg: WsMessage) => void) => () => void;
	let send: (msg: WsMessage) => void;

	beforeEach(() => {
		subscribers = [];
		sentMessages = [];
		subscribe = (cb) => {
			subscribers.push(cb);
			return () => {
				subscribers = subscribers.filter((s) => s !== cb);
			};
		};
		send = (msg) => sentMessages.push(msg);
	});

	function emit(msg: WsMessage) {
		for (const sub of subscribers) sub(msg);
	}

	it("複数 pty_ready でセッションが蓄積される", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24 },
			});
		});
		act(() => {
			emit({
				type: "pty_ready",
				payload: {
					pty_id: 2,
					cols: 120,
					rows: 40,
					label: "dev",
					worktree_path: "/repo",
				},
			});
		});

		expect(result.current.ptySessions).toHaveLength(2);
		expect(result.current.ptySessions[0].ptyId).toBe(1);
		expect(result.current.ptySessions[1].ptyId).toBe(2);
		expect(result.current.ptySessions[1].label).toBe("dev");
		expect(result.current.ptySessions[1].worktreePath).toBe("/repo");
		expect(result.current.activePtyId).toBe(1);
	});

	it("重複 pty_id のセッションは追加されない", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24 },
			});
		});
		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24 },
			});
		});

		expect(result.current.ptySessions).toHaveLength(1);
	});

	it("pty_exit で該当セッションのみ除去される", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24 },
			});
			emit({
				type: "pty_ready",
				payload: { pty_id: 2, cols: 80, rows: 24 },
			});
		});
		act(() => {
			emit({ type: "pty_exit", payload: { pty_id: 1, exit_code: 0 } });
		});

		expect(result.current.ptySessions).toHaveLength(1);
		expect(result.current.ptySessions[0].ptyId).toBe(2);
	});

	it("worktree_select_response でセッションがリセットされる", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24, label: "Terminal 1" },
			});
			emit({
				type: "pty_ready",
				payload: { pty_id: 2, cols: 80, rows: 24, label: "Terminal 2" },
			});
		});

		expect(result.current.ptySessions).toHaveLength(2);

		act(() => {
			emit({
				type: "worktree_select_response",
				payload: { success: true, path: "/new-repo" },
			});
		});

		// worktree切り替えでセッション一覧がリセットされる
		expect(result.current.ptySessions).toHaveLength(0);
		expect(result.current.activePtyId).toBeNull();
	});

	it("worktree_select_response 後に PtyReady で新worktreeのセッションが復元される", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24, label: "Terminal 1" },
			});
		});

		expect(result.current.ptySessions).toHaveLength(1);

		act(() => {
			emit({
				type: "worktree_select_response",
				payload: { success: true, path: "/new-repo" },
			});
		});

		expect(result.current.ptySessions).toHaveLength(0);

		act(() => {
			emit({
				type: "pty_ready",
				payload: {
					pty_id: 3,
					cols: 80,
					rows: 24,
					label: "Terminal 1",
					worktree_path: "/new-repo",
				},
			});
		});

		expect(result.current.ptySessions).toHaveLength(1);
		expect(result.current.ptySessions[0].ptyId).toBe(3);
		expect(result.current.activePtyId).toBe(3);
	});

	it("spawnPty が label 付き pty_spawn_request を送信する", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			result.current.spawnPty("dev-server");
		});

		expect(sentMessages).toHaveLength(1);
		expect(sentMessages[0]).toEqual({
			type: "pty_spawn_request",
			payload: { cols: 80, rows: 24, label: "dev-server" },
		});
	});

	it("killPty が pty_kill_request を送信する", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			result.current.killPty(42);
		});

		expect(sentMessages).toHaveLength(1);
		expect(sentMessages[0]).toEqual({
			type: "pty_kill_request",
			payload: { pty_id: 42 },
		});
	});

	it("spawnPty で label なしでも送信できる", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			result.current.spawnPty();
		});

		expect(sentMessages[0]).toEqual({
			type: "pty_spawn_request",
			payload: { cols: 80, rows: 24, label: undefined },
		});
	});

	it("kind 付き pty_ready でセッションに kind が設定される", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24, kind: "agent" },
			});
		});

		expect(result.current.ptySessions[0].kind).toBe("agent");
	});

	it("kind 未設定で terminal がデフォルトになる", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24 },
			});
		});

		expect(result.current.ptySessions[0].kind).toBe("terminal");
	});

	it("agent セッションで activePtyId が自動設定されない", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24, kind: "agent" },
			});
		});

		expect(result.current.activePtyId).toBeNull();
		expect(result.current.activeAgentPtyId).toBe(1);
	});

	it("agent セッションで activeAgentPtyId が自動設定される", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 10, cols: 80, rows: 24, kind: "agent" },
			});
		});

		expect(result.current.activeAgentPtyId).toBe(10);
	});

	it("terminalSessions と agentSessions がフィルタされる", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24, kind: "terminal" },
			});
			emit({
				type: "pty_ready",
				payload: { pty_id: 2, cols: 80, rows: 24, kind: "agent" },
			});
			emit({
				type: "pty_ready",
				payload: { pty_id: 3, cols: 80, rows: 24 },
			});
		});

		expect(result.current.terminalSessions).toHaveLength(2);
		expect(result.current.agentSessions).toHaveLength(1);
		expect(result.current.agentSessions[0].ptyId).toBe(2);
	});

	it("pty_exit で activeAgentPtyId もクリアされる", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 5, cols: 80, rows: 24, kind: "agent" },
			});
		});

		expect(result.current.activeAgentPtyId).toBe(5);

		act(() => {
			emit({ type: "pty_exit", payload: { pty_id: 5, exit_code: 0 } });
		});

		expect(result.current.activeAgentPtyId).toBeNull();
	});

	it("複数の Agent pty_ready で agentSessions に全て含まれる", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 10, cols: 80, rows: 24, kind: "agent" },
			});
		});
		act(() => {
			emit({
				type: "pty_ready",
				payload: {
					pty_id: 11,
					cols: 80,
					rows: 24,
					kind: "agent",
					label: "claude",
				},
			});
		});

		expect(result.current.agentSessions).toHaveLength(2);
		expect(result.current.agentSessions[0].ptyId).toBe(10);
		expect(result.current.agentSessions[1].ptyId).toBe(11);
		expect(result.current.agentSessions[1].label).toBe("claude");
	});

	it("Agent セッションが追加されると agentSessions がリアルタイムで増加する", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 20, cols: 80, rows: 24, kind: "agent" },
			});
		});

		expect(result.current.agentSessions).toHaveLength(1);

		act(() => {
			emit({
				type: "pty_ready",
				payload: {
					pty_id: 21,
					cols: 80,
					rows: 24,
					kind: "agent",
					label: "new-agent",
				},
			});
		});

		expect(result.current.agentSessions).toHaveLength(2);
		expect(result.current.agentSessions[1].ptyId).toBe(21);
		expect(result.current.agentSessions[1].label).toBe("new-agent");
	});

	it("worktree_select_response で activeAgentPtyId もリセットされる", () => {
		const { result } = renderHook(() => usePtyManagement({ subscribe, send }));

		act(() => {
			emit({
				type: "pty_ready",
				payload: { pty_id: 1, cols: 80, rows: 24, kind: "agent" },
			});
		});

		expect(result.current.activeAgentPtyId).toBe(1);

		act(() => {
			emit({
				type: "worktree_select_response",
				payload: { success: true, path: "/new-repo" },
			});
		});

		expect(result.current.activeAgentPtyId).toBeNull();
	});
});
