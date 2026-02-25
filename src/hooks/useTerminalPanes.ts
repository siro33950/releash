import { invoke } from "@tauri-apps/api/core";
import { useCallback, useRef, useState } from "react";
import {
	closePane,
	countLeaves,
	findNode,
	getAdjacentPane,
	getAllLeaves,
	splitPane,
} from "@/lib/paneTree";
import type {
	PaneLeaf,
	PaneNode,
	SplitDirection,
	TerminalTab,
} from "@/types/terminal-pane";

const MAX_TABS = 8;
const MAX_PANES_PER_TAB = 4;

let tabIdCounter = 0;
function nextTabId() {
	tabIdCounter += 1;
	return `tab-${tabIdCounter}`;
}

let paneIdCounter = 0;
function nextPaneId() {
	paneIdCounter += 1;
	return `pane-${paneIdCounter}`;
}

/** テスト用: カウンタリセット */
export function _resetIdCounters(): void {
	tabIdCounter = 0;
	paneIdCounter = 0;
}

function createLeaf(label: string): PaneLeaf {
	return {
		type: "leaf",
		id: nextPaneId(),
		label,
		ptyId: null,
	};
}

type NavigationDirection = "left" | "right" | "up" | "down";

export interface UseTerminalPanesReturn {
	tabs: TerminalTab[];
	activeTabId: string;
	setActiveTabId: (id: string) => void;
	addTab: () => void;
	closeTab: (tabId: string) => void;
	splitFocusedPane: (direction: SplitDirection) => void;
	closeFocusedPane: () => void;
	closeSpecificPane: (paneId: string) => void;
	moveFocus: (direction: NavigationDirection) => void;
	setFocusedPane: (paneId: string) => void;
	activeTab: TerminalTab | undefined;
}

export function useTerminalPanes(tabPrefix: string): UseTerminalPanesReturn {
	const tabCounter = useRef(1);

	const [tabs, setTabs] = useState<TerminalTab[]>(() => {
		const pane = createLeaf(`${tabPrefix} 1`);
		return [
			{
				id: nextTabId(),
				label: `${tabPrefix} 1`,
				paneTree: pane,
				focusedPaneId: pane.id,
			},
		];
	});
	const [activeTabId, setActiveTabId] = useState<string>(tabs[0].id);

	const activeTab = tabs.find((t) => t.id === activeTabId);

	const addTab = useCallback(() => {
		tabCounter.current += 1;
		const num = tabCounter.current;
		const label = `${tabPrefix} ${num}`;
		const pane = createLeaf(label);
		const newTab: TerminalTab = {
			id: nextTabId(),
			label,
			paneTree: pane,
			focusedPaneId: pane.id,
		};
		setTabs((prev) => {
			if (prev.length >= MAX_TABS) return prev;
			setActiveTabId(newTab.id);
			return [...prev, newTab];
		});
	}, [tabPrefix]);

	const killPanesInTree = useCallback((tree: PaneNode) => {
		const leaves = getAllLeaves(tree);
		for (const leaf of leaves) {
			if (leaf.ptyId != null) {
				invoke("kill_pty", { ptyId: leaf.ptyId }).catch((err) =>
					console.warn("kill_pty failed:", err),
				);
			}
		}
	}, []);

	const closeTab = useCallback(
		(tabId: string) => {
			setTabs((prev) => {
				if (prev.length <= 1) return prev;
				const tab = prev.find((t) => t.id === tabId);
				if (tab) killPanesInTree(tab.paneTree);
				const next = prev.filter((t) => t.id !== tabId);
				setActiveTabId((currentActive) => {
					if (currentActive !== tabId) return currentActive;
					const idx = prev.findIndex((t) => t.id === tabId);
					const fallback = prev[idx - 1] ?? prev[idx + 1];
					return fallback?.id ?? currentActive;
				});
				return next;
			});
		},
		[killPanesInTree],
	);

	const updateActiveTab = useCallback(
		(updater: (tab: TerminalTab) => TerminalTab) => {
			setTabs((prev) =>
				prev.map((tab) => (tab.id === activeTabId ? updater(tab) : tab)),
			);
		},
		[activeTabId],
	);

	const splitFocusedPane = useCallback(
		(direction: SplitDirection) => {
			updateActiveTab((tab) => {
				if (countLeaves(tab.paneTree) >= MAX_PANES_PER_TAB) return tab;

				const newLeaf = createLeaf(tab.label);
				const newTree = splitPane(
					tab.paneTree,
					tab.focusedPaneId,
					direction,
					newLeaf,
				);
				return {
					...tab,
					paneTree: newTree,
					focusedPaneId: newLeaf.id,
				};
			});
		},
		[updateActiveTab],
	);

	const closeSpecificPane = useCallback((paneId: string) => {
		setTabs((prev) => {
			const tabIndex = prev.findIndex((t) => findNode(t.paneTree, paneId));
			if (tabIndex === -1) return prev;

			const tab = prev[tabIndex];
			const leaf = findNode(tab.paneTree, paneId);
			if (leaf?.type === "leaf" && leaf.ptyId != null) {
				invoke("kill_pty", { ptyId: leaf.ptyId }).catch((err) =>
					console.warn("kill_pty failed:", err),
				);
			}

			const newTree = closePane(tab.paneTree, paneId);

			if (newTree === null) {
				// 最後のペイン → タブを閉じる
				if (prev.length <= 1) return prev;
				const next = prev.filter((_, i) => i !== tabIndex);
				setActiveTabId((currentActive) => {
					if (currentActive !== tab.id) return currentActive;
					const fallback = prev[tabIndex - 1] ?? prev[tabIndex + 1];
					return fallback?.id ?? currentActive;
				});
				return next;
			}

			// フォーカスを兄弟に移動
			let newFocused = tab.focusedPaneId;
			if (paneId === tab.focusedPaneId) {
				const leaves = getAllLeaves(newTree);
				newFocused = leaves[0]?.id ?? tab.focusedPaneId;
			}

			return prev.map((t, i) =>
				i === tabIndex
					? { ...t, paneTree: newTree, focusedPaneId: newFocused }
					: t,
			);
		});
	}, []);

	const closeFocusedPane = useCallback(() => {
		const tab = tabs.find((t) => t.id === activeTabId);
		if (!tab) return;
		closeSpecificPane(tab.focusedPaneId);
	}, [tabs, activeTabId, closeSpecificPane]);

	const moveFocus = useCallback(
		(direction: NavigationDirection) => {
			updateActiveTab((tab) => {
				const adjacent = getAdjacentPane(
					tab.paneTree,
					tab.focusedPaneId,
					direction,
				);
				if (!adjacent) return tab;
				return { ...tab, focusedPaneId: adjacent };
			});
		},
		[updateActiveTab],
	);

	const setFocusedPane = useCallback(
		(paneId: string) => {
			updateActiveTab((tab) => {
				if (tab.focusedPaneId === paneId) return tab;
				return { ...tab, focusedPaneId: paneId };
			});
		},
		[updateActiveTab],
	);

	return {
		tabs,
		activeTabId,
		setActiveTabId,
		addTab,
		closeTab,
		splitFocusedPane,
		closeFocusedPane,
		closeSpecificPane,
		moveFocus,
		setFocusedPane,
		activeTab,
	};
}
