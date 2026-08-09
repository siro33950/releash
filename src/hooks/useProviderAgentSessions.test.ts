import { invoke } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { notifyProviderAgentSessionChanged } from "@/lib/providerAgentSessionEvents";
import type { ProviderAgentSessionItem } from "@/types/provider-agent-session";
import { useProviderAgentSessions } from "./useProviderAgentSessions";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));

const tauriListeners = vi.hoisted(
	() =>
		new Map<string, (event: { payload: { worktreePath?: string } }) => void>(),
);

vi.mock("@tauri-apps/api/event", () => ({
	listen: (
		eventName: string,
		handler: (event: { payload: { worktreePath?: string } }) => void,
	) => {
		tauriListeners.set(eventName, handler);
		return Promise.resolve(() => {
			if (tauriListeners.get(eventName) === handler) {
				tauriListeners.delete(eventName);
			}
		});
	},
}));

const mockInvoke = vi.mocked(invoke);

function session(id: string): ProviderAgentSessionItem {
	return {
		id,
		workspaceIdentity: "/repo/worktree",
		worktreePath: "/repo/worktree",
		provider: "claude",
		origin: { kind: "standalone" },
		lifecycle: "open",
		activity: "idle",
		lastExitAbnormal: false,
		providerSessionId: `provider-${id}`,
		transcriptRef: null,
		operations: {
			canArchive: true,
			canRestore: false,
			canDelete: false,
		},
	};
}

describe("useProviderAgentSessions", () => {
	beforeEach(() => {
		mockInvoke.mockReset();
		tauriListeners.clear();
	});

	it("Archived一覧はbackendへlifecycleを明示する", async () => {
		mockInvoke.mockResolvedValueOnce({
			items: [],
			nextAfterSessionId: null,
		});

		renderHook(() => useProviderAgentSessions("/repo/worktree", "archived"));

		await waitFor(() => {
			expect(mockInvoke).toHaveBeenCalledWith("list_provider_agent_sessions", {
				workspaceIdentity: "/repo/worktree",
				lifecycle: "archived",
				origin: "standalone",
				limit: 100,
			});
		});
	});

	it("cursorから次のpageを読み既存pageへ追記する", async () => {
		mockInvoke
			.mockResolvedValueOnce({
				items: [session("agent-1")],
				nextAfterSessionId: "agent-1",
			})
			.mockResolvedValueOnce({
				items: [session("agent-2")],
				nextAfterSessionId: null,
			});
		const { result } = renderHook(() =>
			useProviderAgentSessions("/repo/worktree"),
		);

		await waitFor(() => expect(result.current.items).toHaveLength(1));
		await act(() => result.current.loadMore());

		expect(mockInvoke).toHaveBeenNthCalledWith(
			2,
			"list_provider_agent_sessions",
			{
				workspaceIdentity: "/repo/worktree",
				origin: "standalone",
				limit: 100,
				afterSessionId: "agent-1",
			},
		);
		expect(result.current.items.map((item) => item.id)).toEqual([
			"agent-1",
			"agent-2",
		]);
		expect(result.current.hasMore).toBe(false);
	});

	it("refresh失敗はerrorを設定しcursorをリセットする", async () => {
		mockInvoke
			.mockResolvedValueOnce({
				items: [session("agent-1")],
				nextAfterSessionId: "agent-1",
			})
			.mockRejectedValueOnce(new Error("list failed"));
		const { result } = renderHook(() =>
			useProviderAgentSessions("/repo/worktree"),
		);
		await waitFor(() => expect(result.current.hasMore).toBe(true));

		await act(() => result.current.refresh());

		expect(result.current.error).toBe("list failed");
		expect(result.current.hasMore).toBe(false);
		expect(result.current.loading).toBe(false);
		expect(result.current.items.map((item) => item.id)).toEqual(["agent-1"]);
	});

	it("workspaceがnullになると状態をリセットしbackendを呼ばない", async () => {
		mockInvoke.mockResolvedValueOnce({
			items: [session("agent-1")],
			nextAfterSessionId: "agent-1",
		});
		const { result, rerender } = renderHook(
			({ workspace }: { workspace: string | null }) =>
				useProviderAgentSessions(workspace),
			{ initialProps: { workspace: "/repo/worktree" as string | null } },
		);
		await waitFor(() => expect(result.current.items).toHaveLength(1));
		mockInvoke.mockClear();

		rerender({ workspace: null });

		await waitFor(() => expect(result.current.items).toHaveLength(0));
		expect(result.current.loading).toBe(false);
		expect(result.current.error).toBeNull();
		expect(result.current.hasMore).toBe(false);
		expect(mockInvoke).not.toHaveBeenCalled();
	});

	it("provider-agent-session-refreshはworktreePath一致時のみ再読込する", async () => {
		mockInvoke.mockResolvedValue({ items: [], nextAfterSessionId: null });
		renderHook(() => useProviderAgentSessions("/repo/worktree"));
		await waitFor(() => expect(mockInvoke).toHaveBeenCalledTimes(1));

		act(() => notifyProviderAgentSessionChanged("/repo/other"));

		expect(mockInvoke).toHaveBeenCalledTimes(1);

		act(() => notifyProviderAgentSessionChanged("/repo/worktree"));

		await waitFor(() => expect(mockInvoke).toHaveBeenCalledTimes(2));
	});

	it("backendのprovider-agent-session-changedはworktreePath一致時のみ再読込する", async () => {
		mockInvoke.mockResolvedValue({ items: [], nextAfterSessionId: null });
		renderHook(() => useProviderAgentSessions("/repo/worktree"));
		await waitFor(() => expect(mockInvoke).toHaveBeenCalledTimes(1));
		const emitBackendEvent = tauriListeners.get(
			"provider-agent-session-changed",
		);
		expect(emitBackendEvent).toBeDefined();

		act(() => emitBackendEvent?.({ payload: { worktreePath: "/repo/other" } }));

		expect(mockInvoke).toHaveBeenCalledTimes(1);

		act(() =>
			emitBackendEvent?.({ payload: { worktreePath: "/repo/worktree" } }),
		);

		await waitFor(() => expect(mockInvoke).toHaveBeenCalledTimes(2));
	});

	it("unmountでbackendイベント購読を解除する", async () => {
		mockInvoke.mockResolvedValue({ items: [], nextAfterSessionId: null });
		const { unmount } = renderHook(() =>
			useProviderAgentSessions("/repo/worktree"),
		);
		await waitFor(() =>
			expect(tauriListeners.has("provider-agent-session-changed")).toBe(true),
		);

		unmount();

		await waitFor(() =>
			expect(tauriListeners.has("provider-agent-session-changed")).toBe(false),
		);
	});
});
