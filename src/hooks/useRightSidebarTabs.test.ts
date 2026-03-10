import { act, renderHook } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { useRightSidebarTabs } from "./useRightSidebarTabs";

const baseParams = {
	centerTab: "editor",
	activeView: "explorer",
	onActiveViewChange: vi.fn(),
};

describe("useRightSidebarTabs", () => {
	describe("mode 導出", () => {
		it("centerTab が 'workflow' のとき mode は 'workflow'", () => {
			const { result } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, centerTab: "workflow" }),
			);
			expect(result.current.mode).toBe("workflow");
		});

		it("centerTab が 'editor' のとき mode は 'editor'", () => {
			const { result } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, centerTab: "editor" }),
			);
			expect(result.current.mode).toBe("editor");
		});

		it("centerTab が未知の値のとき mode は 'editor'", () => {
			const { result } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, centerTab: "unknown" }),
			);
			expect(result.current.mode).toBe("editor");
		});
	});

	describe("editor モードのタブ状態", () => {
		it("初期 activeTopTab が 'explorer'", () => {
			const { result } = renderHook(() => useRightSidebarTabs(baseParams));
			expect(result.current.activeTopTab).toBe("explorer");
		});

		it("初期 activeBottomTab が 'terminal'", () => {
			const { result } = renderHook(() => useRightSidebarTabs(baseParams));
			expect(result.current.activeBottomTab).toBe("terminal");
		});

		it("handleTopTabChange で activeView 同期が行われる", () => {
			const onActiveViewChange = vi.fn();
			const { result } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, onActiveViewChange }),
			);

			act(() => result.current.handleTopTabChange("changes"));
			expect(onActiveViewChange).toHaveBeenCalledWith("git");

			act(() => result.current.handleTopTabChange("search"));
			expect(onActiveViewChange).toHaveBeenCalledWith("search");
		});

		it("handleBottomTabChange でタブが切り替わる", () => {
			const { result } = renderHook(() => useRightSidebarTabs(baseParams));

			act(() => result.current.handleBottomTabChange("comment"));
			expect(result.current.activeBottomTab).toBe("comment");
		});
	});

	describe("workflow モードのタブ状態", () => {
		const workflowParams = { ...baseParams, centerTab: "workflow" };

		it("初期 activeTopTab が 'plan-timeline'", () => {
			const { result } = renderHook(() => useRightSidebarTabs(workflowParams));
			expect(result.current.activeTopTab).toBe("plan-timeline");
		});

		it("初期 activeBottomTab が 'terminal'", () => {
			const { result } = renderHook(() => useRightSidebarTabs(workflowParams));
			expect(result.current.activeBottomTab).toBe("terminal");
		});

		it("handleTopTabChange で onActiveViewChange が呼ばれない", () => {
			const onActiveViewChange = vi.fn();
			const { result } = renderHook(() =>
				useRightSidebarTabs({
					...workflowParams,
					onActiveViewChange,
				}),
			);

			act(() => result.current.handleTopTabChange("plan-comment"));
			expect(result.current.activeTopTab).toBe("plan-comment");
			expect(onActiveViewChange).not.toHaveBeenCalled();
		});
	});

	describe("モード間のタブ独立性", () => {
		it("editor と workflow のタブ状態が互いに影響しない", () => {
			let centerTab = "editor";
			const { result, rerender } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, centerTab }),
			);

			// editor モードで changes に変更
			act(() => result.current.handleTopTabChange("changes"));

			// workflow に切り替え → plan-timeline のまま
			centerTab = "workflow";
			rerender();
			expect(result.current.activeTopTab).toBe("plan-timeline");

			// workflow で plan-comment に変更
			act(() => result.current.handleTopTabChange("plan-comment"));
			expect(result.current.activeTopTab).toBe("plan-comment");

			// editor に戻る → changes が維持されている
			centerTab = "editor";
			rerender();
			// activeView が "explorer" なので editorTopTab (changes ではなく viewToTabMap で解決)
			// editorTopTab は "changes" に更新されているが、activeView が "explorer" なので editorTopTab が使われる
			expect(result.current.activeTopTab).toBe("changes");
		});
	});

	describe("activeView → RightTopTab マッピング", () => {
		it("activeView='git' → activeTopTab='changes'", () => {
			const { result } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, activeView: "git" }),
			);
			expect(result.current.activeTopTab).toBe("changes");
		});

		it("activeView='search' → activeTopTab='search'", () => {
			const { result } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, activeView: "search" }),
			);
			expect(result.current.activeTopTab).toBe("search");
		});

		it("activeView='pr' → activeTopTab='pr'", () => {
			const { result } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, activeView: "pr" }),
			);
			expect(result.current.activeTopTab).toBe("pr");
		});

		it("activeView='symbols' → activeTopTab='symbols'", () => {
			const { result } = renderHook(() =>
				useRightSidebarTabs({ ...baseParams, activeView: "symbols" }),
			);
			expect(result.current.activeTopTab).toBe("symbols");
		});

		it("未知の activeView は editorTopTab にフォールバック", () => {
			const { result } = renderHook(() =>
				useRightSidebarTabs({
					...baseParams,
					activeView: "unknown-view",
				}),
			);
			expect(result.current.activeTopTab).toBe("explorer");
		});
	});
});
