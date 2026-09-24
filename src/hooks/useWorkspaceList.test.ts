import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { WorkspaceListSnapshotDto } from "@/generated/client_types";
import { workspaceListSnapshot } from "@/test/workspaceList";
import { useWorkspaceList } from "./useWorkspaceList";

const mocks = vi.hoisted(() => ({
	invoke: vi.fn(),
	subscribe: vi.fn(),
	stop: vi.fn(),
}));
vi.mock("@/lib/client", () => ({
	invokeClient: mocks.invoke,
	subscribeState: mocks.subscribe,
}));
let deliver: (value: WorkspaceListSnapshotDto) => void;
beforeEach(() => {
	vi.clearAllMocks();
	mocks.invoke.mockResolvedValue(undefined);
	mocks.subscribe.mockImplementation((_target, receiver) => {
		deliver = receiver;
		return mocks.stop;
	});
});
afterEach(() => vi.useRealTimers());

describe("useWorkspaceList", () => {
	it("初期状態と変更は購読から届き単発取得も監視要求も行わない", () => {
		const { result, unmount, rerender } = renderHook(() => useWorkspaceList());
		expect(result.current.snapshot).toBeNull();
		const initial = workspaceListSnapshot();
		act(() => deliver(initial));
		expect(result.current.snapshot).toBe(initial);
		const previous = result.current;
		rerender();
		expect(result.current).toBe(previous);
		const next = workspaceListSnapshot();
		next.repositories[0].branches[0].has_pr = true;
		act(() => deliver(next));
		expect(result.current.snapshot).toBe(next);
		expect(mocks.invoke).not.toHaveBeenCalled();
		expect(mocks.subscribe).toHaveBeenCalledTimes(1);
		unmount();
		expect(mocks.stop).toHaveBeenCalledOnce();
	});
	it.each(["repository", "worktree", "all"])(
		"%sの失敗範囲と前回の一覧をdaemonの値どおり保持する",
		(scope) => {
			const { result } = renderHook(() => useWorkspaceList());
			const failed = workspaceListSnapshot();
			const status =
				scope === "all"
					? failed.status
					: scope === "repository"
						? failed.repositories[0].status
						: failed.repositories[0].worktrees[0].status;
			Object.assign(status, { state: "refreshFailed", error: "offline" });
			act(() => deliver(failed));
			expect(result.current.snapshot).toBe(failed);
			const recovered = workspaceListSnapshot();
			act(() => deliver(recovered));
			expect(result.current.snapshot).toBe(recovered);
		},
	);
	it("初回取得失敗と正常な空を区別する", () => {
		const { result } = renderHook(() => useWorkspaceList());
		act(() =>
			deliver({
				generation: 1,
				status: { loaded: false, state: "initialFailed", error: "offline" },
				repositories: [],
			}),
		);
		expect(result.current.snapshot?.status.state).toBe("initialFailed");
		act(() =>
			deliver({
				generation: 2,
				status: { loaded: true, state: "empty", error: null },
				repositories: [],
			}),
		);
		expect(result.current.snapshot?.status.state).toBe("empty");
	});
	it("更新要求は結果を表示へ使わず失敗後も前回の一覧とRefreshを維持する", async () => {
		const { result } = renderHook(() => useWorkspaceList());
		const previous = workspaceListSnapshot();
		act(() => deliver(previous));
		mocks.invoke.mockRejectedValueOnce(new Error("offline"));
		await act(() => result.current.refresh());
		expect(result.current.snapshot).toBe(previous);
		expect(result.current.requestError?.message).toBe("offline");
		await act(() => result.current.refresh());
		expect(result.current.requestError).toBeNull();
		expect(result.current.snapshot).toBe(previous);
		expect(mocks.invoke.mock.calls).toEqual([
			["refresh_workspaces", {}],
			["refresh_workspaces", {}],
		]);
	});
	it("削除中も定期実行とwindow通知から取り直さない", async () => {
		vi.useFakeTimers();
		renderHook(() => useWorkspaceList());
		const snapshot = workspaceListSnapshot();
		snapshot.repositories[0].branches[0].is_deleting = true;
		act(() => deliver(snapshot));
		await act(async () => {
			window.dispatchEvent(new Event("branch-list-refresh"));
			window.dispatchEvent(new Event("workspace-tree-refresh"));
			await vi.advanceTimersByTimeAsync(240_000);
		});
		expect(mocks.invoke).not.toHaveBeenCalled();
	});
});
