import { act, renderHook } from "@testing-library/react";
import { createElement, type ReactNode } from "react";
import { beforeEach, expect, it, vi } from "vitest";
import type { WorkspaceTreeSelectionSnapshotDto } from "@/generated/client_types";
import { subscribeState } from "@/lib/client";
import { workspaceListSnapshot } from "@/test/workspaceList";
import { WorkspaceListContext } from "./useWorkspaceList";
import { useWorkspaceTreeNodes } from "./useWorkspaceTreeNodes";

vi.mock("@/lib/client", () => ({ subscribeState: vi.fn() }));
let snapshot = workspaceListSnapshot();
const stop = vi.fn();
let receive: (value: WorkspaceTreeSelectionSnapshotDto) => void;
function wrapper({ children }: { children: ReactNode }) {
	return createElement(
		WorkspaceListContext.Provider,
		{ value: { snapshot, requestError: null, refresh: vi.fn() } },
		children,
	);
}
beforeEach(() => {
	vi.clearAllMocks();
	snapshot = workspaceListSnapshot();
	vi.mocked(subscribeState).mockImplementation((_target, next) => {
		receive = next;
		return stop;
	});
});
it("ツリーと履歴と失敗範囲は一覧の購読値を表示する", () => {
	const { result, rerender } = renderHook(
		() => useWorkspaceTreeNodes("/repo"),
		{ wrapper },
	);
	expect(result.current.nodes).toEqual(
		snapshot.repositories[0].worktrees[0].snapshot?.nodes,
	);
	snapshot.repositories[0].worktrees[0].status = {
		loaded: true,
		state: "refreshFailed",
		error: "offline",
	};
	rerender();
	expect(result.current.error).toBe("offline");
	expect(result.current.loaded).toBe(true);
	expect(subscribeState).not.toHaveBeenCalled();
});
it("archive後の選択照合は購読し同じ対象の変更も受け取る", () => {
	const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"), {
		wrapper,
	});
	act(() => result.current.beginArchiveReconciliation("node"));
	expect(subscribeState).toHaveBeenCalledWith(
		{ kind: "selection", args: ["/repo", "node"] },
		expect.any(Function),
	);
	const tree = snapshot.repositories[0].worktrees[0].snapshot;
	if (!tree) throw new Error("Missing fixture snapshot");
	act(() =>
		receive({ snapshot: tree, reconciliation: { selectionInSnapshot: true } }),
	);
	expect(result.current.reconciliationEvent?.selectionInSnapshot).toBe(true);
	act(() =>
		receive({ snapshot: tree, reconciliation: { selectionInSnapshot: false } }),
	);
	const event = result.current.reconciliationEvent;
	if (!event) throw new Error("Missing reconciliation event");
	expect(event.selectionInSnapshot).toBe(false);
	expect(result.current.isReconciliationEventCurrent(event, "node")).toBe(true);
});
it("選択が変わると古い照合を破棄し購読を解除する", () => {
	const { result } = renderHook(() => useWorkspaceTreeNodes("/repo"), {
		wrapper,
	});
	act(() => result.current.beginArchiveReconciliation("old"));
	const old = receive;
	const tree = snapshot.repositories[0].worktrees[0].snapshot;
	if (!tree) throw new Error("Missing fixture snapshot");
	act(() => result.current.synchronizeSelectedNodeId("new"));
	act(() =>
		old({
			snapshot: tree,
			reconciliation: { selectionInSnapshot: false },
		}),
	);
	expect(result.current.reconciliationEvent).toBeNull();
	expect(stop).toHaveBeenCalledOnce();
});
it("worktree変更とunmountで照合の購読を解除する", () => {
	const { result, rerender, unmount } = renderHook(
		({ path }) => useWorkspaceTreeNodes(path),
		{ wrapper, initialProps: { path: "/repo" } },
	);
	act(() => result.current.beginArchiveReconciliation("node"));
	rerender({ path: "/other" });
	expect(stop).toHaveBeenCalledOnce();
	expect(result.current.loaded).toBe(false);
	unmount();
});
