import { useCallback, useReducer, useRef } from "react";

const AGENT_TAB_ID = "agent-tab";

export interface EditorTab {
	id: string;
	path: string | null;
	name: string;
	component: "agent" | "editor";
	isDirty: boolean;
	closable: boolean;
	draggable: boolean;
}

export interface EditorLayoutState {
	tabs: EditorTab[];
	activeTabId: string;
}

type EditorLayoutAction =
	| { type: "ADD_TAB"; path: string; name: string; isDirty: boolean }
	| { type: "REMOVE_TAB"; path: string }
	| { type: "SELECT_TAB"; tabId: string }
	| { type: "UPDATE_DIRTY"; path: string; isDirty: boolean }
	| { type: "REORDER"; fromIndex: number; toIndex: number };

function tabIdFromPath(path: string): string {
	return `editor:${path}`;
}

export function pathFromTabId(tabId: string): string | null {
	return tabId.startsWith("editor:") ? tabId.slice("editor:".length) : null;
}

function createInitialState(): EditorLayoutState {
	return {
		tabs: [
			{
				id: AGENT_TAB_ID,
				path: null,
				name: "Agent",
				component: "agent",
				isDirty: false,
				closable: false,
				draggable: false,
			},
		],
		activeTabId: AGENT_TAB_ID,
	};
}

function reducer(
	state: EditorLayoutState,
	action: EditorLayoutAction,
): EditorLayoutState {
	switch (action.type) {
		case "ADD_TAB": {
			const tabId = tabIdFromPath(action.path);
			const existing = state.tabs.find((t) => t.id === tabId);
			if (existing) {
				return { ...state, activeTabId: tabId };
			}
			const newTab: EditorTab = {
				id: tabId,
				path: action.path,
				name: action.name,
				component: "editor",
				isDirty: action.isDirty,
				closable: true,
				draggable: true,
			};
			return {
				tabs: [...state.tabs, newTab],
				activeTabId: tabId,
			};
		}
		case "REMOVE_TAB": {
			const tabId = tabIdFromPath(action.path);
			const idx = state.tabs.findIndex((t) => t.id === tabId);
			if (idx === -1) return state;
			const nextTabs = state.tabs.filter((t) => t.id !== tabId);
			let nextActive = state.activeTabId;
			if (state.activeTabId === tabId) {
				const prev = state.tabs[idx - 1];
				const next = state.tabs[idx + 1];
				nextActive = (next ?? prev)?.id ?? AGENT_TAB_ID;
			}
			return { tabs: nextTabs, activeTabId: nextActive };
		}
		case "SELECT_TAB": {
			if (state.activeTabId === action.tabId) return state;
			return { ...state, activeTabId: action.tabId };
		}
		case "UPDATE_DIRTY": {
			const tabId = tabIdFromPath(action.path);
			const tabs = state.tabs.map((t) =>
				t.id === tabId ? { ...t, isDirty: action.isDirty } : t,
			);
			return { ...state, tabs };
		}
		case "REORDER": {
			const { fromIndex, toIndex } = action;
			if (
				fromIndex === toIndex ||
				fromIndex < 0 ||
				toIndex < 0 ||
				fromIndex >= state.tabs.length ||
				toIndex >= state.tabs.length
			)
				return state;
			const tabs = [...state.tabs];
			const [moved] = tabs.splice(fromIndex, 1);
			tabs.splice(toIndex, 0, moved);
			return { ...state, tabs };
		}
	}
}

export interface UseEditorLayoutReturn {
	tabs: EditorTab[];
	activeTabId: string;
	addTab: (path: string, name: string, isDirty: boolean) => void;
	removeTab: (path: string) => void;
	selectTab: (path: string) => void;
	selectTabById: (tabId: string) => void;
	getActiveTabPath: () => string | null;
	updateTabDirty: (path: string, isDirty: boolean) => void;
	reorderTabs: (fromIndex: number, toIndex: number) => void;
	closeTab: (path: string) => void;
}

export function useEditorLayout(
	onTabClose?: (path: string) => boolean,
): UseEditorLayoutReturn {
	const [state, dispatch] = useReducer(reducer, undefined, createInitialState);
	const stateRef = useRef(state);
	stateRef.current = state;

	const addTab = useCallback((path: string, name: string, isDirty: boolean) => {
		dispatch({ type: "ADD_TAB", path, name, isDirty });
	}, []);

	const removeTab = useCallback((path: string) => {
		dispatch({ type: "REMOVE_TAB", path });
	}, []);

	const selectTab = useCallback((path: string) => {
		const tabId = tabIdFromPath(path);
		dispatch({ type: "SELECT_TAB", tabId });
	}, []);

	const selectTabById = useCallback((tabId: string) => {
		dispatch({ type: "SELECT_TAB", tabId });
	}, []);

	const getActiveTabPath = useCallback((): string | null => {
		return pathFromTabId(stateRef.current.activeTabId);
	}, []);

	const updateTabDirty = useCallback((path: string, isDirty: boolean) => {
		dispatch({ type: "UPDATE_DIRTY", path, isDirty });
	}, []);

	const reorderTabs = useCallback((fromIndex: number, toIndex: number) => {
		dispatch({ type: "REORDER", fromIndex, toIndex });
	}, []);

	const closeTab = useCallback(
		(path: string) => {
			const shouldBlock = onTabClose?.(path) ?? false;
			if (!shouldBlock) {
				dispatch({ type: "REMOVE_TAB", path });
			}
		},
		[onTabClose],
	);

	return {
		tabs: state.tabs,
		activeTabId: state.activeTabId,
		addTab,
		removeTab,
		selectTab,
		selectTabById,
		getActiveTabPath,
		updateTabDirty,
		reorderTabs,
		closeTab,
	};
}

export { AGENT_TAB_ID };
