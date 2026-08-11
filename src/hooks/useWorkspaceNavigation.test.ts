import { act, renderHook } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import { useWorkspaceNavigation } from "./useWorkspaceNavigation";

describe("useWorkspaceNavigation", () => {
	it("openWorktreeTab は同一 rootPath を再利用し異なる rootPath は追加する", () => {
		const { result } = renderHook(() => useWorkspaceNavigation());

		act(() => {
			result.current.openWorktreeTab("/repo/a", "main", "repo");
		});
		act(() => {
			result.current.openWorktreeTab("/repo/b", "feat/b", "repo");
		});
		act(() => {
			result.current.openWorktreeTab("/repo/a", "ignored", "other");
		});

		expect(result.current.worktrees).toHaveLength(2);
		expect(result.current.worktrees[0]).toMatchObject({
			id: "/repo/a",
			rootPath: "/repo/a",
			branchName: "main",
			repoName: "repo",
		});
		expect(result.current.worktrees[1]).toMatchObject({
			id: "/repo/b",
			rootPath: "/repo/b",
			branchName: "feat/b",
		});
		expect(result.current.selectedWorktreeId).toBe("/repo/a");
	});

	it("close_quit_workspace_close_is_view_only", () => {
		const { result } = renderHook(() => useWorkspaceNavigation());

		act(() => {
			result.current.openWorktreeTab("/repo/active", "main", "repo");
		});
		act(() => {
			result.current.openWorktreeTab("/repo/other", "feature", "repo");
		});
		const retainedWorkspace = result.current.worktrees[0];

		act(() => {
			result.current.closeWorktreeTab("/repo/other");
		});

		expect(result.current.worktrees).toEqual([retainedWorkspace]);
		expect(result.current.worktrees[0]).toBe(retainedWorkspace);
		expect(result.current.selectedWorktreeId).toBe("/repo/active");
	});
});
