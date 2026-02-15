import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useWorkspaceTabs } from "./useWorkspaceTabs";

vi.mock("@tauri-apps/api/event", () => ({
	listen: vi.fn().mockResolvedValue(() => {}),
}));

describe("useWorkspaceTabs", () => {
	it("should initialize with kanban tab only", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		expect(result.current.tabs).toHaveLength(1);
		expect(result.current.tabs[0]).toEqual({ type: "kanban", id: "kanban" });
		expect(result.current.activeTabId).toBe("kanban");
	});

	it("should open a worktree tab and set it active", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.openWorktreeTab("/path/to/worktree", "feat/test");
		});
		expect(result.current.tabs).toHaveLength(2);
		expect(result.current.tabs[1]).toEqual({
			type: "worktree",
			id: "/path/to/worktree",
			rootPath: "/path/to/worktree",
			branchName: "feat/test",
		});
		expect(result.current.activeTabId).toBe("/path/to/worktree");
	});

	it("should not duplicate tab for same rootPath", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.openWorktreeTab("/path/to/worktree", "feat/test");
		});
		act(() => {
			result.current.openWorktreeTab("/path/to/worktree", "feat/test");
		});
		expect(result.current.tabs).toHaveLength(2);
		expect(result.current.activeTabId).toBe("/path/to/worktree");
	});

	it("should focus existing tab when opening same rootPath", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.openWorktreeTab("/path/a", "branch-a");
		});
		act(() => {
			result.current.openWorktreeTab("/path/b", "branch-b");
		});
		expect(result.current.activeTabId).toBe("/path/b");
		act(() => {
			result.current.openWorktreeTab("/path/a", "branch-a");
		});
		expect(result.current.activeTabId).toBe("/path/a");
	});

	it("should use rootPath last segment as fallback branchName", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.openWorktreeTab("/path/to/my-project");
		});
		const tab = result.current.tabs[1];
		expect(tab.type === "worktree" && tab.branchName).toBe("my-project");
	});

	it("should close a worktree tab", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.openWorktreeTab("/path/a", "branch-a");
		});
		act(() => {
			result.current.closeWorktreeTab("/path/a");
		});
		expect(result.current.tabs).toHaveLength(1);
		expect(result.current.activeTabId).toBe("kanban");
	});

	it("should fallback to previous tab when closing active tab", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.openWorktreeTab("/path/a", "branch-a");
		});
		act(() => {
			result.current.openWorktreeTab("/path/b", "branch-b");
		});
		act(() => {
			result.current.closeWorktreeTab("/path/b");
		});
		expect(result.current.activeTabId).toBe("/path/a");
	});

	it("should not close kanban tab", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.closeWorktreeTab("kanban");
		});
		expect(result.current.tabs).toHaveLength(1);
	});

	it("should switch to kanban", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.openWorktreeTab("/path/a", "branch-a");
		});
		expect(result.current.activeTabId).toBe("/path/a");
		act(() => {
			result.current.switchToKanban();
		});
		expect(result.current.activeTabId).toBe("kanban");
	});

	it("should set active tab by id", () => {
		const { result } = renderHook(() => useWorkspaceTabs());
		act(() => {
			result.current.openWorktreeTab("/path/a", "branch-a");
		});
		act(() => {
			result.current.setActiveTab("kanban");
		});
		expect(result.current.activeTabId).toBe("kanban");
	});

	describe("reorderTabs", () => {
		it("should reorder tabs by moving fromId to toId position", () => {
			const { result } = renderHook(() => useWorkspaceTabs());
			act(() => {
				result.current.openWorktreeTab("/path/a", "branch-a");
			});
			act(() => {
				result.current.openWorktreeTab("/path/b", "branch-b");
			});
			// tabs: [kanban, /path/a, /path/b]
			act(() => {
				result.current.reorderTabs("/path/a", "/path/b");
			});
			expect(result.current.tabs.map((t) => t.id)).toEqual([
				"kanban",
				"/path/b",
				"/path/a",
			]);
		});

		it("should do nothing when fromId === toId", () => {
			const { result } = renderHook(() => useWorkspaceTabs());
			act(() => {
				result.current.openWorktreeTab("/path/a", "branch-a");
			});
			act(() => {
				result.current.reorderTabs("/path/a", "/path/a");
			});
			expect(result.current.tabs.map((t) => t.id)).toEqual([
				"kanban",
				"/path/a",
			]);
		});

		it("should do nothing when id is not found", () => {
			const { result } = renderHook(() => useWorkspaceTabs());
			act(() => {
				result.current.openWorktreeTab("/path/a", "branch-a");
			});
			act(() => {
				result.current.reorderTabs("/nonexistent", "/path/a");
			});
			expect(result.current.tabs.map((t) => t.id)).toEqual([
				"kanban",
				"/path/a",
			]);
		});
	});
});
