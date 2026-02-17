import type { Action } from "flexlayout-react";
import {
	Actions,
	DockLocation,
	type IJsonModel,
	type ITabRenderValues,
	Model,
	type TabNode,
	TabSetNode,
} from "flexlayout-react";
import { useCallback, useMemo, useRef } from "react";

const EDITOR_TABSET_ID = "editor-tabs";

function createInitialJson(): IJsonModel {
	return {
		global: {
			tabEnableClose: true,
			tabEnableDrag: true,
			tabEnableRename: false,
			tabSetEnableMaximize: false,
			tabSetEnableDeleteWhenEmpty: true,
			enableEdgeDock: false,
			splitterSize: 1,
			splitterExtra: 4,
		},
		layout: {
			type: "row",
			weight: 100,
			children: [
				{
					type: "tabset",
					id: EDITOR_TABSET_ID,
					weight: 100,
					enableDeleteWhenEmpty: false,
					enableTabScrollbar: true,
					children: [],
				},
			],
		},
	};
}

export interface UseEditorLayoutReturn {
	model: Model;
	addTab: (path: string, name: string, isDirty: boolean) => void;
	removeTab: (path: string) => void;
	selectTab: (path: string) => void;
	getActiveTabPath: () => string | null;
	updateTabDirty: (path: string, isDirty: boolean) => void;
	onAction: (action: Action) => Action | undefined;
}

function tabIdFromPath(path: string): string {
	return `editor:${path}`;
}

export function pathFromTabId(tabId: string): string | null {
	return tabId.startsWith("editor:") ? tabId.slice("editor:".length) : null;
}

export function useEditorLayout(
	onTabClose?: (path: string) => boolean,
): UseEditorLayoutReturn {
	const model = useMemo(() => Model.fromJson(createInitialJson()), []);
	const modelRef = useRef(model);
	modelRef.current = model;

	const addTab = useCallback((path: string, name: string, isDirty: boolean) => {
		const tabId = tabIdFromPath(path);
		const existing = modelRef.current.getNodeById(tabId);
		if (existing) {
			modelRef.current.doAction(Actions.selectTab(tabId));
			return;
		}
		modelRef.current.doAction(
			Actions.addNode(
				{
					type: "tab",
					id: tabId,
					name,
					component: "editor",
					config: { filePath: path, isDirty },
				},
				EDITOR_TABSET_ID,
				DockLocation.CENTER,
				-1,
				true,
			),
		);
	}, []);

	const removeTab = useCallback((path: string) => {
		const tabId = tabIdFromPath(path);
		const existing = modelRef.current.getNodeById(tabId);
		if (existing) {
			modelRef.current.doAction(Actions.deleteTab(tabId));
		}
	}, []);

	const selectTab = useCallback((path: string) => {
		const tabId = tabIdFromPath(path);
		const existing = modelRef.current.getNodeById(tabId);
		if (existing) {
			modelRef.current.doAction(Actions.selectTab(tabId));
		}
	}, []);

	const getActiveTabPath = useCallback((): string | null => {
		const tabset = modelRef.current.getNodeById(EDITOR_TABSET_ID);
		if (!tabset) return null;
		if (!(tabset instanceof TabSetNode)) return null;
		const children = tabset.getChildren() as TabNode[];
		const selected = tabset.getSelected();
		if (selected == null || selected < 0 || selected >= children.length)
			return null;
		const activeTab = children[selected];
		return pathFromTabId(activeTab.getId());
	}, []);

	const updateTabDirty = useCallback((path: string, isDirty: boolean) => {
		const tabId = tabIdFromPath(path);
		const existing = modelRef.current.getNodeById(tabId);
		if (existing) {
			modelRef.current.doAction(
				Actions.updateNodeAttributes(tabId, {
					config: { filePath: path, isDirty },
				}),
			);
		}
	}, []);

	const onAction = useCallback(
		(action: Action): Action | undefined => {
			if (action.type === Actions.DELETE_TAB) {
				const tabId = action.data.node as string | undefined;
				if (tabId) {
					const path = pathFromTabId(tabId);
					if (path) {
						const shouldBlock = onTabClose?.(path) ?? false;
						return shouldBlock ? undefined : action;
					}
				}
			}
			return action;
		},
		[onTabClose],
	);

	return {
		model,
		addTab,
		removeTab,
		selectTab,
		getActiveTabPath,
		updateTabDirty,
		onAction,
	};
}

export { EDITOR_TABSET_ID };

export type { ITabRenderValues, TabNode };
