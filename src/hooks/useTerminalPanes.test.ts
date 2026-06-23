import { invoke } from "@tauri-apps/api/core";
import { act, renderHook, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { countLeaves, getAllLeaves } from "@/lib/paneTree";
import {
	_clearTabStateCache,
	_resetIdCounters,
	useTerminalPanes,
} from "./useTerminalPanes";

const mockListen = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({
	invoke: vi.fn().mockResolvedValue(undefined),
}));

vi.mock("@tauri-apps/api/event", () => ({
	listen: (...args: unknown[]) => mockListen(...args),
}));

describe("useTerminalPanes", () => {
	beforeEach(() => {
		vi.clearAllMocks();
		mockListen.mockResolvedValue(vi.fn());
		_resetIdCounters();
		_clearTabStateCache();
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

	it("タブ上限(MAX_TABS)を超えて追加できない", () => {
		const { result } = renderHook(() => useTerminalPanes("Terminal"));
		// 初期1タブ + 7タブ追加 = 8タブ (MAX_TABS)
		for (let i = 0; i < 7; i++) {
			act(() => result.current.addTab());
		}
		expect(result.current.tabs).toHaveLength(8);
		const lastLabel = result.current.tabs[7].label;

		// 上限到達後の追加は no-op
		act(() => result.current.addTab());
		expect(result.current.tabs).toHaveLength(8);
		// ラベル番号が飛ばないことも確認
		expect(result.current.tabs[7].label).toBe(lastLabel);
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

	it("再マウント時にペインラベルが同じ値を生成する", () => {
		const { result, unmount } = renderHook(() => useTerminalPanes("Terminal"));
		const firstLabel = result.current.tabs[0].paneTree;
		expect(firstLabel.type).toBe("leaf");
		if (firstLabel.type === "leaf") {
			expect(firstLabel.label).toBe("Terminal 1");
		}
		unmount();

		// 再マウント: ローカルカウンターがリセットされ同じラベルが生成される
		_resetIdCounters();
		const { result: result2 } = renderHook(() => useTerminalPanes("Terminal"));
		const secondLabel = result2.current.tabs[0].paneTree;
		expect(secondLabel.type).toBe("leaf");
		if (secondLabel.type === "leaf") {
			expect(secondLabel.label).toBe("Terminal 1");
		}
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

	describe("moveTabToPane", () => {
		it("タブを別タブのペインに移動（タブ減少・ペイン追加・フォーカス・activeTab変更）", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.addTab());
			expect(result.current.tabs).toHaveLength(2);

			const sourceTab = result.current.tabs[0];
			const targetTab = result.current.tabs[1];
			const targetPaneId = targetTab.focusedPaneId;
			const sourceLeaf = getAllLeaves(sourceTab.paneTree)[0];

			act(() =>
				result.current.moveTabToPane(sourceTab.id, targetPaneId, "vertical"),
			);

			expect(result.current.tabs).toHaveLength(1);
			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(2);
			expect(result.current.tabs[0].focusedPaneId).toBe(sourceLeaf.id);
			expect(result.current.activeTabId).toBe(targetTab.id);
		});

		it("同一タブへのドロップは no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));

			const tab = result.current.tabs[0];
			const paneId = tab.focusedPaneId;

			act(() => result.current.moveTabToPane(tab.id, paneId, "vertical"));

			expect(result.current.tabs).toHaveLength(1);
			expect(result.current.tabs[0].paneTree.type).toBe("leaf");
		});

		it("複数ペインを持つタブの移動は no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));
			act(() => result.current.addTab());

			const sourceTab = result.current.tabs[0];
			const targetTab = result.current.tabs[1];

			act(() =>
				result.current.moveTabToPane(
					sourceTab.id,
					targetTab.focusedPaneId,
					"vertical",
				),
			);

			expect(result.current.tabs).toHaveLength(2);
		});

		it("存在しないソースタブは no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.addTab());

			const targetPaneId = result.current.tabs[1].focusedPaneId;

			act(() =>
				result.current.moveTabToPane(
					"nonexistent-tab",
					targetPaneId,
					"vertical",
				),
			);

			expect(result.current.tabs).toHaveLength(2);
		});

		it("存在しないターゲットペインは no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.addTab());

			const sourceTabId = result.current.tabs[0].id;

			act(() =>
				result.current.moveTabToPane(
					sourceTabId,
					"nonexistent-pane",
					"vertical",
				),
			);

			expect(result.current.tabs).toHaveLength(2);
		});

		it("kill_pty が呼ばれないこと（PTY維持）", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.addTab());

			const sourceTab = result.current.tabs[0];
			const targetPaneId = result.current.tabs[1].focusedPaneId;

			act(() =>
				result.current.moveTabToPane(sourceTab.id, targetPaneId, "vertical"),
			);

			expect(invoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
		});

		it("水平方向への移動", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.addTab());

			const sourceTab = result.current.tabs[0];
			const targetPaneId = result.current.tabs[1].focusedPaneId;

			act(() =>
				result.current.moveTabToPane(sourceTab.id, targetPaneId, "horizontal"),
			);

			expect(result.current.tabs).toHaveLength(1);
			const tree = result.current.tabs[0].paneTree;
			expect(tree.type).toBe("container");
			if (tree.type === "container") {
				expect(tree.direction).toBe("horizontal");
			}
		});

		it("ターゲットタブがペイン上限の場合は no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			// ターゲットタブを4ペインにする
			act(() => result.current.splitFocusedPane("vertical"));
			act(() => result.current.splitFocusedPane("vertical"));
			act(() => result.current.splitFocusedPane("vertical"));
			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(4);

			act(() => result.current.addTab());
			const sourceTab = result.current.tabs[1];
			const targetPaneId = result.current.tabs[0].focusedPaneId;

			act(() =>
				result.current.moveTabToPane(sourceTab.id, targetPaneId, "vertical"),
			);

			expect(result.current.tabs).toHaveLength(2);
			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(4);
		});
	});

	describe("movePaneToTab", () => {
		it("ペインをタブに分離（新タブ作成・ソースタブのペイン数減少）", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));

			const tab = result.current.tabs[0];
			const tree = tab.paneTree;
			if (tree.type !== "container") throw new Error("should be container");
			const paneToBreak = tree.children[1];

			act(() => result.current.movePaneToTab(paneToBreak.id));

			expect(result.current.tabs).toHaveLength(2);
			// ソースタブは1ペインに
			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(1);
			// 新タブは分離されたペインの1ペイン
			expect(countLeaves(result.current.tabs[1].paneTree)).toBe(1);
			expect(result.current.tabs[1].paneTree.id).toBe(paneToBreak.id);
			// アクティブタブは新タブ
			expect(result.current.activeTabId).toBe(result.current.tabs[1].id);
		});

		it("ソースタブの直後に新タブが挿入される", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.addTab());
			// Tab 0 を分割
			act(() => result.current.setActiveTabId(result.current.tabs[0].id));
			act(() => result.current.splitFocusedPane("vertical"));

			const tab0 = result.current.tabs[0];
			const tree = tab0.paneTree;
			if (tree.type !== "container") throw new Error("should be container");
			const paneToBreak = tree.children[1];

			act(() => result.current.movePaneToTab(paneToBreak.id));

			// Tab 0, 新タブ, Tab 1 の順
			expect(result.current.tabs).toHaveLength(3);
			expect(result.current.tabs[1].paneTree.id).toBe(paneToBreak.id);
		});

		it("単一ペインのタブでは no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			const paneId = result.current.tabs[0].focusedPaneId;

			act(() => result.current.movePaneToTab(paneId));

			expect(result.current.tabs).toHaveLength(1);
		});

		it("タブ上限(8)の場合は no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			// 8タブまで追加
			for (let i = 0; i < 7; i++) {
				act(() => result.current.addTab());
			}
			expect(result.current.tabs).toHaveLength(8);

			// Tab 0 を分割
			act(() => result.current.setActiveTabId(result.current.tabs[0].id));
			act(() => result.current.splitFocusedPane("vertical"));
			const tab0 = result.current.tabs[0];
			const tree = tab0.paneTree;
			if (tree.type !== "container") throw new Error("should be container");

			act(() => result.current.movePaneToTab(tree.children[1].id));

			expect(result.current.tabs).toHaveLength(8);
		});

		it("PTY が kill されないこと（セッション維持）", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));

			const tab = result.current.tabs[0];
			const tree = tab.paneTree;
			if (tree.type !== "container") throw new Error("should be container");

			act(() => result.current.movePaneToTab(tree.children[1].id));

			expect(invoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
		});

		it("存在しないペインIDは no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));

			act(() => result.current.movePaneToTab("nonexistent-pane"));

			expect(result.current.tabs).toHaveLength(1);
		});

		it("フォーカスペインを分離した場合、ソースタブのフォーカスが移動する", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));

			const tab = result.current.tabs[0];
			const focusedPaneId = tab.focusedPaneId;
			const tree = tab.paneTree;
			if (tree.type !== "container") throw new Error("should be container");
			const otherPaneId = tree.children.find((c) => c.id !== focusedPaneId)?.id;

			act(() => result.current.movePaneToTab(focusedPaneId));

			// ソースタブのフォーカスは残ったペインに移動
			expect(result.current.tabs[0].focusedPaneId).toBe(otherPaneId);
		});
	});

	describe("movePaneInTab", () => {
		it("ペインをグリッド内で移動（ペイン数不変）", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));
			act(() => result.current.splitFocusedPane("vertical"));

			const tab = result.current.tabs[0];
			const tree = tab.paneTree;
			if (tree.type !== "container") throw new Error("should be container");

			const paneA = tree.children[0].id;
			const paneC = tree.children[2].id;

			act(() =>
				result.current.movePaneInTab(paneA, paneC, "horizontal", false),
			);

			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(3);
			expect(result.current.tabs[0].focusedPaneId).toBe(paneA);
		});

		it("同じペインへの移動は no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));

			const tab = result.current.tabs[0];
			const tree = tab.paneTree;
			if (tree.type !== "container") throw new Error("should be container");
			const paneId = tree.children[0].id;

			act(() => result.current.movePaneInTab(paneId, paneId, "vertical"));

			// ツリー構造に変化なし
			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(2);
		});

		it("insertBefore=true で左/上に挿入", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));

			const tab = result.current.tabs[0];
			const tree = tab.paneTree;
			if (tree.type !== "container") throw new Error("should be container");
			const paneA = tree.children[0].id;
			const paneB = tree.children[1].id;

			// paneA を paneB の前（horizontal）に移動
			act(() => result.current.movePaneInTab(paneA, paneB, "horizontal", true));

			const newTree = result.current.tabs[0].paneTree;
			expect(countLeaves(newTree)).toBe(2);
			// paneA が paneB の上に配置（horizontal container の最初の子）
			if (newTree.type === "container") {
				const firstLeaf = getAllLeaves(newTree)[0];
				expect(firstLeaf.id).toBe(paneA);
			}
		});

		it("異なるタブ間のペインは no-op", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));
			act(() => result.current.addTab());
			act(() => result.current.splitFocusedPane("vertical"));

			const tab0Tree = result.current.tabs[0].paneTree;
			const tab1Tree = result.current.tabs[1].paneTree;
			if (tab0Tree.type !== "container" || tab1Tree.type !== "container")
				throw new Error("should be containers");

			const tab0Pane = tab0Tree.children[0].id;
			const tab1Pane = tab1Tree.children[0].id;

			act(() => result.current.movePaneInTab(tab0Pane, tab1Pane, "vertical"));

			// 変化なし
			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(2);
			expect(countLeaves(result.current.tabs[1].paneTree)).toBe(2);
		});

		it("PTY が kill されないこと", () => {
			const { result } = renderHook(() => useTerminalPanes("Terminal"));
			act(() => result.current.splitFocusedPane("vertical"));

			const tab = result.current.tabs[0];
			const tree = tab.paneTree;
			if (tree.type !== "container") throw new Error("should be container");

			act(() =>
				result.current.movePaneInTab(
					tree.children[0].id,
					tree.children[1].id,
					"horizontal",
				),
			);

			expect(invoke).not.toHaveBeenCalledWith("kill_pty", expect.anything());
		});
	});

	describe("Rust lifecycle mirror", () => {
		it("updatePaneSessionKey stores the PTY id with the session key", () => {
			const { result } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);
			const paneId = result.current.tabs[0].focusedPaneId;

			act(() => result.current.updatePaneSessionKey(paneId, "key-active", 41));

			const leaf = getAllLeaves(result.current.tabs[0].paneTree)[0];
			expect(leaf.sessionKey).toBe("key-active");
			expect(leaf.ptyId).toBe(41);
		});

		it("closeSpecificPane kills the PTY stored on an inactive pane", () => {
			const { result } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);

			act(() => result.current.splitFocusedPane("vertical"));
			const leaves = getAllLeaves(result.current.tabs[0].paneTree);
			act(() =>
				result.current.updatePaneSessionKey(leaves[0].id, "key-old", 41),
			);
			act(() =>
				result.current.updatePaneSessionKey(leaves[1].id, "key-active", 42),
			);
			vi.mocked(invoke).mockClear();

			act(() => result.current.closeSpecificPane(leaves[0].id));

			expect(invoke).toHaveBeenCalledWith("kill_pty", { ptyId: 41 });
			const remainingLeaves = getAllLeaves(result.current.tabs[0].paneTree);
			expect(remainingLeaves).toHaveLength(1);
			expect(remainingLeaves[0].sessionKey).toBe("key-active");
		});

		it("closeTab kills PTYs stored on an inactive tab", () => {
			const { result } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);

			const inactiveTab = result.current.tabs[0];
			const inactivePane = getAllLeaves(inactiveTab.paneTree)[0];
			act(() =>
				result.current.updatePaneSessionKey(
					inactivePane.id,
					"key-inactive-tab",
					43,
				),
			);
			act(() => result.current.addTab());
			vi.mocked(invoke).mockClear();

			act(() => result.current.closeTab(inactiveTab.id));

			expect(invoke).toHaveBeenCalledWith("kill_pty", { ptyId: 43 });
			expect(result.current.tabs).toHaveLength(1);
			expect(result.current.tabs[0].id).not.toBe(inactiveTab.id);
		});

		it("pty-evicted removes the matching inactive pane from cached state", async () => {
			const { result } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);

			act(() => result.current.splitFocusedPane("vertical"));
			const leaves = getAllLeaves(result.current.tabs[0].paneTree);
			act(() => result.current.updatePaneSessionKey(leaves[0].id, "key-old"));
			act(() =>
				result.current.updatePaneSessionKey(leaves[1].id, "key-active"),
			);

			await waitFor(() => {
				expect(mockListen).toHaveBeenCalledWith(
					"pty-evicted",
					expect.any(Function),
				);
			});
			const listener = mockListen.mock.calls.find(
				(call: unknown[]) => call[0] === "pty-evicted",
			)?.[1] as (event: {
				payload: { pty_id: number; session_key: string; reason: string };
			}) => void;

			act(() => {
				listener({
					payload: {
						pty_id: 1,
						session_key: "key-old",
						reason: "idle",
					},
				});
			});

			const remainingLeaves = getAllLeaves(result.current.tabs[0].paneTree);
			expect(remainingLeaves).toHaveLength(1);
			expect(remainingLeaves[0].sessionKey).toBe("key-active");
		});

		it("global pty-evicted mirror removes cached panes while unmounted", async () => {
			const { result, unmount } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);

			act(() => result.current.splitFocusedPane("vertical"));
			const leaves = getAllLeaves(result.current.tabs[0].paneTree);
			act(() => result.current.updatePaneSessionKey(leaves[0].id, "key-old"));
			act(() =>
				result.current.updatePaneSessionKey(leaves[1].id, "key-active"),
			);

			await waitFor(() => {
				expect(
					mockListen.mock.calls.filter(
						(call: unknown[]) => call[0] === "pty-evicted",
					),
				).toHaveLength(2);
			});
			const listeners = mockListen.mock.calls
				.filter((call: unknown[]) => call[0] === "pty-evicted")
				.map((call: unknown[]) => call[1]) as Array<
				(event: {
					payload: { pty_id: number; session_key: string; reason: string };
				}) => void
			>;

			unmount();
			act(() => {
				listeners[1]({
					payload: {
						pty_id: 1,
						session_key: "key-old",
						reason: "idle",
					},
				});
			});

			const { result: remounted } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);
			const remainingLeaves = getAllLeaves(remounted.current.tabs[0].paneTree);
			expect(remainingLeaves).toHaveLength(1);
			expect(remainingLeaves[0].sessionKey).toBe("key-active");
		});

		it("mount reconciliation removes cached panes missing from Rust registry", async () => {
			vi.mocked(invoke).mockImplementation((command) => {
				if (command === "list_pty_sessions") {
					return Promise.resolve([
						{ session_key: "key-old" },
						{ session_key: "key-active" },
					]);
				}
				return Promise.resolve(undefined);
			});

			const { result, unmount } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);

			act(() => result.current.splitFocusedPane("vertical"));
			const leaves = getAllLeaves(result.current.tabs[0].paneTree);
			act(() => result.current.updatePaneSessionKey(leaves[0].id, "key-old"));
			act(() =>
				result.current.updatePaneSessionKey(leaves[1].id, "key-active"),
			);
			await waitFor(() => {
				expect(invoke).toHaveBeenCalledWith("list_pty_sessions");
			});
			unmount();

			vi.mocked(invoke).mockImplementation((command) => {
				if (command === "list_pty_sessions") {
					return Promise.resolve([{ session_key: "key-active" }]);
				}
				return Promise.resolve(undefined);
			});

			const { result: remounted } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);

			await waitFor(() => {
				const remainingLeaves = getAllLeaves(
					remounted.current.tabs[0].paneTree,
				);
				expect(remainingLeaves).toHaveLength(1);
				expect(remainingLeaves[0].sessionKey).toBe("key-active");
			});
		});

		it("removePendingPane rolls back a split pane before PTY initialization succeeds", () => {
			const { result } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);

			act(() => result.current.splitFocusedPane("vertical"));
			const pendingPaneId = result.current.activeTab?.focusedPaneId;
			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(2);

			act(() => result.current.removePendingPane(pendingPaneId ?? ""));

			expect(countLeaves(result.current.tabs[0].paneTree)).toBe(1);
		});

		it("removePendingPane closes a pending tab when it was the only pane", () => {
			const { result } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);

			act(() => result.current.addTab());
			const pendingPaneId = result.current.activeTab?.focusedPaneId;
			expect(result.current.tabs).toHaveLength(2);

			act(() => result.current.removePendingPane(pendingPaneId ?? ""));

			expect(result.current.tabs).toHaveLength(1);
		});

		it("removePendingPane keeps panes that already have a session key", () => {
			const { result } = renderHook(() =>
				useTerminalPanes("Terminal", "/repo::Terminal"),
			);
			const paneId = result.current.tabs[0].focusedPaneId;
			act(() => result.current.updatePaneSessionKey(paneId, "key-active"));

			act(() => result.current.removePendingPane(paneId));

			expect(result.current.tabs).toHaveLength(1);
			expect(getAllLeaves(result.current.tabs[0].paneTree)[0].sessionKey).toBe(
				"key-active",
			);
		});
	});
});
