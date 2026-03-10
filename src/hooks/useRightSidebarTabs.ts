import { useCallback, useState } from "react";
import type { RightBottomTab, RightTopTab } from "@/types/sidebar";

interface UseRightSidebarTabsParams {
	centerTab: string;
	activeView: string;
	onActiveViewChange: (view: string) => void;
}

interface UseRightSidebarTabsReturn {
	mode: "workflow" | "editor";
	activeTopTab: RightTopTab;
	activeBottomTab: RightBottomTab;
	handleTopTabChange: (tab: RightTopTab) => void;
	handleBottomTabChange: (tab: RightBottomTab) => void;
}

const viewToTabMap: Record<string, RightTopTab> = {
	git: "changes",
	search: "search",
	pr: "pr",
	symbols: "symbols",
};

export function useRightSidebarTabs({
	centerTab,
	activeView,
	onActiveViewChange,
}: UseRightSidebarTabsParams): UseRightSidebarTabsReturn {
	const mode = centerTab === "workflow" ? "workflow" : "editor";

	const [editorTopTab, setEditorTopTab] = useState<RightTopTab>("explorer");
	const [workflowTopTab, setWorkflowTopTab] =
		useState<RightTopTab>("plan-timeline");
	const [editorBottomTab, setEditorBottomTab] =
		useState<RightBottomTab>("terminal");
	const [workflowBottomTab, setWorkflowBottomTab] =
		useState<RightBottomTab>("terminal");

	// editor モードでは activeView → RightTopTab マッピングを優先
	const editorActiveTopTab: RightTopTab =
		viewToTabMap[activeView] ?? editorTopTab;

	const activeTopTab =
		mode === "workflow" ? workflowTopTab : editorActiveTopTab;
	const activeBottomTab =
		mode === "workflow" ? workflowBottomTab : editorBottomTab;

	const handleTopTabChange = useCallback(
		(tab: RightTopTab) => {
			if (mode === "workflow") {
				setWorkflowTopTab(tab);
			} else {
				setEditorTopTab(tab);
				const view = tab === "changes" ? "git" : tab;
				onActiveViewChange(view);
			}
		},
		[mode, onActiveViewChange],
	);

	const handleBottomTabChange = useCallback(
		(tab: RightBottomTab) => {
			if (mode === "workflow") setWorkflowBottomTab(tab);
			else setEditorBottomTab(tab);
		},
		[mode],
	);

	return {
		mode,
		activeTopTab,
		activeBottomTab,
		handleTopTabChange,
		handleBottomTabChange,
	};
}
