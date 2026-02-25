import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { _resetIdCounters, useTerminalPanes } from "./useTerminalPanes";

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));

describe("useTerminalPanes", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		_resetIdCounters();
	});

	it("初期状態で1タブ1ペイン", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		expect(result.current.tabs).toHaveLength(1);
		expect(result.current.tabs[0].paneTree.type).toBe("leaf");
		expect(result.current.tabs[0].label).toBe("Terminal 1");
	});

	it("タブ追加", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.addTab());
		expect(result.current.tabs).toHaveLength(2);
		expect(result.current.tabs[1].label).toBe("Terminal 2");
	});

	it("タブ閉じる", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.addTab());
		expect(result.current.tabs).toHaveLength(2);
		act(() => result.current.closeTab(result.current.tabs[0].id));
		expect(result.current.tabs).toHaveLength(1);
	});

	it("最後のタブは閉じられない", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.closeTab(result.current.tabs[0].id));
		expect(result.current.tabs).toHaveLength(1);
	});

	it("フォーカスペインを垂直分割", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.splitFocusedPane("vertical"));

		const tree = result.current.activeTab?.paneTree;
		expect(tree?.type).toBe("container");
		if (tree?.type === "container") {
			expect(tree.children).toHaveLength(2);
			expect(tree.direction).toBe("vertical");
		}
	});

	it("フォーカスペインを水平分割", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.splitFocusedPane("horizontal"));

		const tree = result.current.activeTab?.paneTree;
		expect(tree?.type).toBe("container");
		if (tree?.type === "container") {
			expect(tree.children).toHaveLength(2);
			expect(tree.direction).toBe("horizontal");
		}
	});

	it("最大4ペインまで分割可能", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.splitFocusedPane("vertical"));
		act(() => result.current.splitFocusedPane("vertical"));
		act(() => result.current.splitFocusedPane("vertical"));
		// 4ペイン目以降は追加されない
		act(() => result.current.splitFocusedPane("vertical"));

		const tree = result.current.activeTab?.paneTree;
		expect(tree?.type).toBe("container");
		if (tree?.type === "container") {
			expect(tree.children).toHaveLength(4);
		}
	});

	it("ペインを閉じる", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.splitFocusedPane("vertical"));

		const focusedId = result.current.activeTab?.focusedPaneId;
		expect(focusedId).toBeDefined();

		act(() => result.current.closeFocusedPane());
		expect(result.current.activeTab?.paneTree.type).toBe("leaf");
	});

	it("フォーカス移動", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.splitFocusedPane("vertical"));

		const tree = result.current.activeTab?.paneTree;
		if (tree?.type !== "container") throw new Error("should be container");

		const firstPaneId = tree.children[0].id;
		const secondPaneId = tree.children[1].id;

		// 分割後はnewPaneにフォーカス
		expect(result.current.activeTab?.focusedPaneId).toBe(secondPaneId);

		// 左に移動
		act(() => result.current.moveFocus("left"));
		expect(result.current.activeTab?.focusedPaneId).toBe(firstPaneId);

		// 右に移動
		act(() => result.current.moveFocus("right"));
		expect(result.current.activeTab?.focusedPaneId).toBe(secondPaneId);
	});

	it("フォーカスペインの手動設定", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		act(() => result.current.splitFocusedPane("vertical"));

		const tree = result.current.activeTab?.paneTree;
		if (tree?.type !== "container") throw new Error("should be container");

		const firstPaneId = tree.children[0].id;
		act(() => result.current.setFocusedPane(firstPaneId));
		expect(result.current.activeTab?.focusedPaneId).toBe(firstPaneId);
	});
});
