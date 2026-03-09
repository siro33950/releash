export function worktreeNameFromPath(rootPath: string): string {
	return rootPath.split("/").pop() ?? rootPath;
}

export interface WorkspaceState {
	version: 1;
	tabs: {
		editors: { path: string; name: string }[];
		activeEditorPath: string | null;
	};
	layout: {
		centerTab: "editor" | "workflow";
		activeView: string;
		leftNavCollapsed: boolean;
		rightCollapsed: boolean;
		rightBottomCollapsed: boolean;
		rightBottomActiveTab?: "terminal" | "review";
		workflowPanelRatios?: [number, number];
	};
}

export interface InternalWorktreeState {
	tabs: { path: string; name: string }[];
	activeEditorPath: string | null;
	activeView: string;
	rightBottomCollapsed: boolean;
	rightBottomActiveTab: string;
	workflowPanelRatios?: [number, number];
}

export function buildWorkspaceState(
	internal: InternalWorktreeState,
	centerTab: string,
	leftNavVisible: boolean,
	rightVisible: boolean,
): WorkspaceState {
	return {
		version: 1,
		tabs: {
			editors: internal.tabs,
			activeEditorPath: internal.activeEditorPath,
		},
		layout: {
			centerTab: centerTab as "editor" | "workflow",
			activeView: internal.activeView,
			leftNavCollapsed: !leftNavVisible,
			rightCollapsed: !rightVisible,
			rightBottomCollapsed: internal.rightBottomCollapsed,
			rightBottomActiveTab: internal.rightBottomActiveTab as
				| "terminal"
				| "review",
			workflowPanelRatios: internal.workflowPanelRatios,
		},
	};
}

export function migrateWorkspaceState(state: WorkspaceState): WorkspaceState {
	if ((state.layout.centerTab as string) === "agent") {
		return {
			...state,
			layout: {
				...state.layout,
				centerTab: "workflow",
			},
		};
	}
	return state;
}
