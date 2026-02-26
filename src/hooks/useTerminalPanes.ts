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
	moveTabToPane: (
		sourceTabId: string,
		targetPaneId: string,
		direction: SplitDirection,
	) => void;
	movePaneToTab: (paneId: string) => void;
	movePaneInTab: (
		paneId: string,
		targetPaneId: string,
		direction: SplitDirection,
		insertBefore?: boolean,
	) => void;
	activeTab: TerminalTab | undefined;
}

export function useTerminalPanes(tabPrefix: string): UseTerminalPanesReturn {
	const tabCounter = useRef(1);
	const paneNameCounterRef = useRef(1);
	const tabsLengthRef = useRef(1);

	const createLeaf = useCallback((): PaneLeaf => {
		paneNameCounterRef.current += 1;
		return {
			type: "leaf",
			id: nextPaneId(),
			label: `${tabPrefix} ${paneNameCounterRef.current}`,
			ptyId: null,
		};
	}, [tabPrefix]);

	const [tabs, setTabs] = useState<TerminalTab[]>(() => {
		const pane: PaneLeaf = {
			type: "leaf",
			id: nextPaneId(),
			label: `${tabPrefix} 1`,
			ptyId: null,
		};
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
	tabsLengthRef.current = tabs.length;

	const activeTab = tabs.find((t) => t.id === activeTabId);

	const addTab = useCallback(() => {
		if (tabsLengthRef.current >= MAX_TABS) return;
		tabCounter.current += 1;
		const num = tabCounter.current;
		const label = `${tabPrefix} ${num}`;
		const pane = createLeaf();
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
	}, [tabPrefix, createLeaf]);

	const closeTab = useCallback((tabId: string) => {
		setTabs((prev) => {
			if (prev.length <= 1) return prev;
			const next = prev.filter((t) => t.id !== tabId);
			setActiveTabId((currentActive) => {
				if (currentActive !== tabId) return currentActive;
				const idx = prev.findIndex((t) => t.id === tabId);
				const fallback = prev[idx - 1] ?? prev[idx + 1];
				return fallback?.id ?? currentActive;
			});
			return next;
		});
	}, []);

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
			const newLeaf = createLeaf();
			updateActiveTab((tab) => {
				if (countLeaves(tab.paneTree) >= MAX_PANES_PER_TAB) return tab;

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
		[updateActiveTab, createLeaf],
	);

	const closeSpecificPane = useCallback((paneId: string) => {
		setTabs((prev) => {
			const tabIndex = prev.findIndex((t) => findNode(t.paneTree, paneId));
			if (tabIndex === -1) return prev;

			const tab = prev[tabIndex];

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
				if (!findNode(tab.paneTree, paneId)) return tab;
				return { ...tab, focusedPaneId: paneId };
			});
		},
		[updateActiveTab],
	);

	const moveTabToPane = useCallback(
		(sourceTabId: string, targetPaneId: string, direction: SplitDirection) => {
			setTabs((prev) => {
				const sourceTab = prev.find((t) => t.id === sourceTabId);
				if (!sourceTab) return prev;

				const sourceLeaves = getAllLeaves(sourceTab.paneTree);
				if (sourceLeaves.length !== 1) return prev;
				const sourceLeaf = sourceLeaves[0];

				const targetTabIndex = prev.findIndex((t) =>
					findNode(t.paneTree, targetPaneId),
				);
				if (targetTabIndex === -1) return prev;
				const targetTab = prev[targetTabIndex];

				if (sourceTab.id === targetTab.id) return prev;
				if (countLeaves(targetTab.paneTree) >= MAX_PANES_PER_TAB) return prev;

				const newTargetTree = splitPane(
					targetTab.paneTree,
					targetPaneId,
					direction,
					sourceLeaf,
				);

				const next = prev
					.filter((t) => t.id !== sourceTabId)
					.map((t) =>
						t.id === targetTab.id
							? { ...t, paneTree: newTargetTree, focusedPaneId: sourceLeaf.id }
							: t,
					);

				setActiveTabId(targetTab.id);
				return next;
			});
		},
		[],
	);

	const movePaneToTab = useCallback(
		(paneId: string) => {
			tabCounter.current += 1;
			const newTabId = nextTabId();
			const newTabLabel = `${tabPrefix} ${tabCounter.current}`;

			setTabs((prev) => {
				if (prev.length >= MAX_TABS) return prev;

				const tabIndex = prev.findIndex((t) => findNode(t.paneTree, paneId));
				if (tabIndex === -1) return prev;

				const tab = prev[tabIndex];
				if (countLeaves(tab.paneTree) <= 1) return prev;

				const leaf = findNode(tab.paneTree, paneId);
				if (!leaf || leaf.type !== "leaf") return prev;

				const newTree = closePane(tab.paneTree, paneId);
				if (!newTree) return prev;

				const remainingLeaves = getAllLeaves(newTree);
				const newFocused =
					tab.focusedPaneId === paneId
						? (remainingLeaves[0]?.id ?? tab.focusedPaneId)
						: tab.focusedPaneId;

				const newTab: TerminalTab = {
					id: newTabId,
					label: newTabLabel,
					paneTree: leaf,
					focusedPaneId: leaf.id,
				};

				const next = prev.map((t, i) =>
					i === tabIndex
						? { ...t, paneTree: newTree, focusedPaneId: newFocused }
						: t,
				);
				next.splice(tabIndex + 1, 0, newTab);
				setActiveTabId(newTabId);
				return next;
			});
		},
		[tabPrefix],
	);

	const movePaneInTab = useCallback(
		(
			paneId: string,
			targetPaneId: string,
			direction: SplitDirection,
			insertBefore = false,
		) => {
			setTabs((prev) => {
				if (paneId === targetPaneId) return prev;

				const tabIndex = prev.findIndex((t) => findNode(t.paneTree, paneId));
				if (tabIndex === -1) return prev;

				const tab = prev[tabIndex];
				// 同一タブ内であることを確認
				if (!findNode(tab.paneTree, targetPaneId)) return prev;

				const leaf = findNode(tab.paneTree, paneId);
				if (!leaf || leaf.type !== "leaf") return prev;

				const treeAfterRemoval = closePane(tab.paneTree, paneId);
				if (!treeAfterRemoval) return prev;

				const newTree = splitPane(
					treeAfterRemoval,
					targetPaneId,
					direction,
					leaf,
					insertBefore,
				);

				return prev.map((t, i) =>
					i === tabIndex
						? { ...t, paneTree: newTree, focusedPaneId: leaf.id }
						: t,
				);
			});
		},
		[],
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
		moveTabToPane,
		movePaneToTab,
		movePaneInTab,
		activeTab,
	};
}
