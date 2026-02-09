import { invoke } from "@tauri-apps/api/core";
import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useWorktrees } from "./useWorktrees";

vi.mocked(invoke).mockResolvedValue([]);

describe("useWorktrees", () => {
	it("returns empty array when repoPath is null", async () => {
		const { result } = renderHook(() => useWorktrees(null));
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.worktrees).toEqual([]);
	});

	it("calls list_worktrees with repoPath", async () => {
		const mockWorktrees = [
			{
				name: "main",
				path: "/repo",
				branch: "main",
				is_main: true,
				is_locked: false,
				dirty_count: 0,
				base_branch: null,
			},
		];
		vi.mocked(invoke).mockResolvedValueOnce(mockWorktrees);

		const { result } = renderHook(() => useWorktrees("/repo"));
		await waitFor(() => expect(result.current.loading).toBe(false));
		expect(result.current.worktrees).toEqual(mockWorktrees);
		expect(invoke).toHaveBeenCalledWith("list_worktrees", {
			repoPath: "/repo",
		});
	});
});
