import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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

interface CachedTabState {
	tabs: TerminalTab[];
	activeTabId: string;
	tabCounter: number;
	paneNameCounter: number;
}

interface PtyEvicted {
	pty_id: number;
	session_key: string;
	reason: string;
}

interface PtySessionAvailability {
	unavailable_session_keys: string[];
}

const tabStateCache = new Map<string, CachedTabState>();

/** テスト用: キャッシュクリア */
export function _clearTabStateCache(): void {
	tabStateCache.clear();
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
	updatePaneSessionKey: (
		paneId: string,
		sessionKey: string,
		ptyId?: number,
	) => void;
	markPendingPaneKill: (paneId: string) => void;
	removePendingPane: (paneId: string) => void;
	activeTab: TerminalTab | undefined;
}

export function useTerminalPanes(
	tabPrefix: string,
	cacheKey?: string | null,
): UseTerminalPanesReturn {
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
			sessionKey: null,
		};
	}, [tabPrefix]);

	const [tabs, setTabs] = useState<TerminalTab[]>(() => {
		if (cacheKey) {
			const cached = tabStateCache.get(cacheKey);
			if (cached) {
				tabCounter.current = cached.tabCounter;
				paneNameCounterRef.current = cached.paneNameCounter;
				return cached.tabs;
			}
		}
		const pane: PaneLeaf = {
			type: "leaf",
			id: nextPaneId(),
			label: `${tabPrefix} 1`,
			ptyId: null,
			sessionKey: null,
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
	const [activeTabId, setActiveTabId] = useState<string>(() => {
		if (cacheKey) {
			const cached = tabStateCache.get(cacheKey);
			if (cached) return cached.activeTabId;
		}
		return tabs[0].id;
	});
	tabsLengthRef.current = tabs.length;

	// キャッシュ更新
	useEffect(() => {
		if (!cacheKey) return;
		tabStateCache.set(cacheKey, {
			tabs,
			activeTabId,
			tabCounter: tabCounter.current,
			paneNameCounter: paneNameCounterRef.current,
		});
	}, [cacheKey, tabs, activeTabId]);

	const activeTab = tabs.find((t) => t.id === activeTabId);
	const activeTabIdRef = useRef(activeTabId);
	activeTabIdRef.current = activeTabId;
	const referencedSessionKeySignature = useMemo(
		() => collectReferencedSessionKeySignature(tabs),
		[tabs],
	);

	useEffect(() => {
		if (!cacheKey) return;
		let cancelled = false;
		let unlisten: (() => void) | undefined;

		listen<PtyEvicted>("pty-evicted", (event) => {
			if (cancelled) return;
			setTabs((prev) => {
				const result = removeBackendSelectedSessionPanes(
					prev,
					activeTabIdRef.current,
					[event.payload.session_key],
					false,
				);
				if (!result.changed) return prev;
				if (result.activeTabId !== activeTabIdRef.current) {
					setActiveTabId(result.activeTabId);
				}
				return result.tabs;
			});
		})
			.then((fn) => {
				if (cancelled) {
					fn();
					return;
				}
				unlisten = fn;
			})
			.catch((error) => {
				console.error("Failed to listen for PTY eviction:", error);
			});

		return () => {
			cancelled = true;
			unlisten?.();
		};
	}, [cacheKey]);

	useEffect(() => {
		if (!cacheKey) return;
		const sessionKeys = parseReferencedSessionKeySignature(
			referencedSessionKeySignature,
		);
		if (sessionKeys.length === 0) return;
		let cancelled = false;

		invoke<PtySessionAvailability>("reconcile_pty_sessions", { sessionKeys })
			.then((availability) => {
				if (
					cancelled ||
					!Array.isArray(availability?.unavailable_session_keys)
				) {
					return;
				}
				setTabs((prev) => {
					const result = removeBackendSelectedSessionPanes(
						prev,
						activeTabIdRef.current,
						availability.unavailable_session_keys,
						true,
					);
					if (!result.changed) return prev;
					if (result.activeTabId !== activeTabIdRef.current) {
						setActiveTabId(result.activeTabId);
					}
					return result.tabs;
				});
			})
			.catch((error) => {
				console.error("Failed to reconcile PTY sessions:", error);
			});

		return () => {
			cancelled = true;
		};
	}, [cacheKey, referencedSessionKeySignature]);

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

	const closeTab = useCallback(
		(tabId: string) => {
			const tab = tabs.find((t) => t.id === tabId);
			if (!tab || tabs.length <= 1) return;
			killPaneTreePtys(tab.paneTree);
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
		},
		[tabs],
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

	const closeSpecificPane = useCallback(
		(paneId: string) => {
			const tabIndex = tabs.findIndex((t) => findNode(t.paneTree, paneId));
			if (tabIndex === -1) return;
			const tab = tabs[tabIndex];
			const target = findNode(tab.paneTree, paneId);
			if (target?.type !== "leaf") return;
			const isLastPaneInLastTab =
				countLeaves(tab.paneTree) <= 1 && tabs.length <= 1;
			if (!isLastPaneInLastTab) {
				killPanePty(target);
			}

			setTabs((prev) => {
				const currentTabIndex = prev.findIndex((t) =>
					findNode(t.paneTree, paneId),
				);
				if (currentTabIndex === -1) return prev;

				const currentTab = prev[currentTabIndex];

				const newTree = closePane(currentTab.paneTree, paneId);

				if (newTree === null) {
					// 最後のペイン → タブを閉じる
					if (prev.length <= 1) return prev;
					const next = prev.filter((_, i) => i !== currentTabIndex);
					setActiveTabId((currentActive) => {
						if (currentActive !== currentTab.id) return currentActive;
						const fallback =
							prev[currentTabIndex - 1] ?? prev[currentTabIndex + 1];
						return fallback?.id ?? currentActive;
					});
					return next;
				}

				// フォーカスを兄弟に移動
				let newFocused = currentTab.focusedPaneId;
				if (paneId === currentTab.focusedPaneId) {
					const leaves = getAllLeaves(newTree);
					newFocused = leaves[0]?.id ?? currentTab.focusedPaneId;
				}

				return prev.map((t, i) =>
					i === currentTabIndex
						? { ...t, paneTree: newTree, focusedPaneId: newFocused }
						: t,
				);
			});
		},
		[tabs],
	);

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
				if (leaf?.type !== "leaf") return prev;

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
				if (leaf?.type !== "leaf") return prev;

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

	const updatePaneSessionKey = useCallback(
		(paneId: string, sessionKey: string, ptyId?: number) => {
			setTabs((prev) =>
				prev.map((tab) => {
					const updated = updateNodeSessionKey(
						tab.paneTree,
						paneId,
						sessionKey,
						ptyId,
					);
					return updated === tab.paneTree ? tab : { ...tab, paneTree: updated };
				}),
			);
		},
		[],
	);

	const markPendingPaneKill = useCallback((paneId: string) => {
		setTabs((prev) => markPendingPaneKillById(prev, paneId));
	}, []);

	const removePendingPane = useCallback((paneId: string) => {
		setTabs((prev) => removePendingPaneById(prev, paneId, setActiveTabId));
	}, []);

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
		updatePaneSessionKey,
		markPendingPaneKill,
		removePendingPane,
		activeTab,
	};
}

function updateNodeSessionKey(
	node: PaneNode,
	paneId: string,
	sessionKey: string,
	ptyId?: number,
): PaneNode {
	if (node.type === "leaf") {
		if (node.id === paneId) {
			return {
				...node,
				ptyId: ptyId ?? node.ptyId,
				sessionKey,
				pendingKill: false,
			};
		}
		return node;
	}
	let changed = false;
	const newChildren = node.children.map((child) => {
		const updated = updateNodeSessionKey(child, paneId, sessionKey, ptyId);
		if (updated !== child) changed = true;
		return updated;
	});
	return changed ? { ...node, children: newChildren } : node;
}

function killPaneTreePtys(node: PaneNode): void {
	for (const leaf of getAllLeaves(node)) {
		killPanePty(leaf);
	}
}

function killPanePty(pane: PaneLeaf): void {
	if (pane.ptyId === null) return;
	invoke("kill_pty", { ptyId: pane.ptyId }).catch((error) => {
		console.error("Failed to kill closed terminal PTY:", error);
	});
}

interface PaneRemovalResult {
	tabs: TerminalTab[];
	activeTabId: string;
	changed: boolean;
}

function collectReferencedSessionKeys(tabs: TerminalTab[]): string[] {
	const sessionKeys = new Set<string>();
	for (const tab of tabs) {
		for (const leaf of getAllLeaves(tab.paneTree)) {
			if (leaf.sessionKey !== null) {
				sessionKeys.add(leaf.sessionKey);
			}
		}
	}
	return [...sessionKeys];
}

function collectReferencedSessionKeySignature(tabs: TerminalTab[]): string {
	return JSON.stringify(collectReferencedSessionKeys(tabs).sort());
}

function parseReferencedSessionKeySignature(signature: string): string[] {
	return JSON.parse(signature) as string[];
}

function removeBackendSelectedSessionPanes(
	tabs: TerminalTab[],
	activeTabId: string,
	sessionKeys: string[],
	removeActiveSinglePaneTab: boolean,
): PaneRemovalResult {
	let currentTabs = tabs;
	let currentActiveTabId = activeTabId;
	let changed = false;

	for (const sessionKey of sessionKeys) {
		while (true) {
			const result = removeFirstPaneBySessionKeyFromState(
				currentTabs,
				sessionKey,
				currentActiveTabId,
				removeActiveSinglePaneTab,
			);
			if (!result.changed) break;
			currentTabs = result.tabs;
			currentActiveTabId = result.activeTabId;
			changed = true;
		}
	}

	return {
		tabs: currentTabs,
		activeTabId: currentActiveTabId,
		changed,
	};
}

function removeFirstPaneBySessionKeyFromState(
	tabs: TerminalTab[],
	sessionKey: string,
	activeTabId: string,
	removeActiveSinglePaneTab: boolean,
): PaneRemovalResult {
	const tabIndex = tabs.findIndex((tab) =>
		getAllLeaves(tab.paneTree).some((leaf) => leaf.sessionKey === sessionKey),
	);
	if (tabIndex === -1) {
		return { tabs, activeTabId, changed: false };
	}

	const tab = tabs[tabIndex];
	const target = getAllLeaves(tab.paneTree).find(
		(leaf) => leaf.sessionKey === sessionKey,
	);
	if (!target) {
		return { tabs, activeTabId, changed: false };
	}

	if (countLeaves(tab.paneTree) <= 1) {
		if (tabs.length <= 1) {
			const clearedPane: PaneLeaf = {
				...target,
				ptyId: null,
				sessionKey: null,
				pendingKill: false,
			};
			return {
				tabs: [
					{ ...tab, paneTree: clearedPane, focusedPaneId: clearedPane.id },
				],
				activeTabId,
				changed: true,
			};
		}
		if (tab.id === activeTabId && !removeActiveSinglePaneTab) {
			const clearedPane: PaneLeaf = {
				...target,
				ptyId: null,
				sessionKey: null,
				pendingKill: false,
			};
			return {
				tabs: tabs.map((candidate, index) =>
					index === tabIndex
						? {
								...candidate,
								paneTree: clearedPane,
								focusedPaneId: clearedPane.id,
							}
						: candidate,
				),
				activeTabId,
				changed: true,
			};
		}
		const nextTabs = tabs.filter((_, index) => index !== tabIndex);
		const nextActiveTabId =
			tab.id === activeTabId
				? ((tabs[tabIndex - 1] ?? tabs[tabIndex + 1])?.id ??
					nextTabs[0]?.id ??
					activeTabId)
				: activeTabId;
		return {
			tabs: nextTabs,
			activeTabId: nextActiveTabId,
			changed: true,
		};
	}

	const newTree = closePane(tab.paneTree, target.id);
	if (!newTree) {
		return { tabs, activeTabId, changed: false };
	}
	const leaves = getAllLeaves(newTree);
	const focusedPaneId =
		tab.focusedPaneId === target.id
			? (leaves[0]?.id ?? tab.focusedPaneId)
			: tab.focusedPaneId;

	return {
		tabs: tabs.map((candidate, index) =>
			index === tabIndex
				? { ...candidate, paneTree: newTree, focusedPaneId }
				: candidate,
		),
		activeTabId,
		changed: true,
	};
}

function markPendingPaneKillById(
	tabs: TerminalTab[],
	paneId: string,
): TerminalTab[] {
	let changed = false;
	const nextTabs = tabs.map((tab) => {
		const updated = markPendingPaneKillInTree(tab.paneTree, paneId);
		if (updated === tab.paneTree) return tab;
		changed = true;
		return { ...tab, paneTree: updated };
	});
	return changed ? nextTabs : tabs;
}

function markPendingPaneKillInTree(node: PaneNode, paneId: string): PaneNode {
	if (node.type === "leaf") {
		if (node.id !== paneId || node.pendingKill) return node;
		return { ...node, pendingKill: true };
	}
	let changed = false;
	const children = node.children.map((child) => {
		const updated = markPendingPaneKillInTree(child, paneId);
		if (updated !== child) changed = true;
		return updated;
	});
	return changed ? { ...node, children } : node;
}

function removePendingPaneById(
	tabs: TerminalTab[],
	paneId: string,
	setActiveTabId: (updater: (currentActive: string) => string) => void,
): TerminalTab[] {
	const tabIndex = tabs.findIndex((tab) => findNode(tab.paneTree, paneId));
	if (tabIndex === -1) return tabs;

	const tab = tabs[tabIndex];
	const target = findNode(tab.paneTree, paneId);
	if (
		target?.type !== "leaf" ||
		target.sessionKey !== null ||
		target.ptyId !== null
	) {
		return tabs;
	}

	if (countLeaves(tab.paneTree) <= 1) {
		if (tabs.length <= 1) return tabs;
		const next = tabs.filter((_, index) => index !== tabIndex);
		setActiveTabId((currentActive) => {
			if (currentActive !== tab.id) return currentActive;
			const fallback = tabs[tabIndex - 1] ?? tabs[tabIndex + 1];
			return fallback?.id ?? currentActive;
		});
		return next;
	}

	const newTree = closePane(tab.paneTree, paneId);
	if (!newTree) return tabs;
	const leaves = getAllLeaves(newTree);
	const focusedPaneId =
		tab.focusedPaneId === paneId
			? (leaves[0]?.id ?? tab.focusedPaneId)
			: tab.focusedPaneId;

	return tabs.map((candidate, index) =>
		index === tabIndex
			? { ...candidate, paneTree: newTree, focusedPaneId }
			: candidate,
	);
}
