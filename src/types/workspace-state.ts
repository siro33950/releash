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
		centerTab: "editor" | "agent";
		activeView: string;
		leftNavCollapsed: boolean;
		rightCollapsed: boolean;
		rightBottomCollapsed: boolean;
		rightBottomActiveTab?: "terminal" | "comments";
	};
}

export interface InternalWorktreeState {
	tabs: { path: string; name: string }[];
	activeEditorPath: string | null;
	activeView: string;
	rightBottomCollapsed: boolean;
	rightBottomActiveTab: string;
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
			centerTab: centerTab as "editor" | "agent",
			activeView: internal.activeView,
			leftNavCollapsed: !leftNavVisible,
			rightCollapsed: !rightVisible,
			rightBottomCollapsed: internal.rightBottomCollapsed,
			rightBottomActiveTab: internal.rightBottomActiveTab as
				| "terminal"
				| "comments",
		},
	};
}
