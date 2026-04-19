export function worktreeNameFromPath(rootPath: string): string {
	return rootPath.split("/").pop() ?? rootPath;
}

export type RightBottomActiveTab = "terminal" | "comments";

/**
 * Normalizes a raw `rightBottomActiveTab` value (possibly a legacy string) into
 * the current union type. Legacy `"review"` is migrated to `"comments"`, and
 * any unknown value falls back to `"terminal"`.
 */
export function normalizeRightBottomActiveTab(
	value: string | null | undefined,
): RightBottomActiveTab {
	if (value === "comments" || value === "review") return "comments";
	if (value === "terminal") return "terminal";
	return "terminal";
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
		rightBottomActiveTab?: RightBottomActiveTab;
		reviewCollapsed?: boolean;
	};
}

export interface InternalWorktreeState {
	tabs: { path: string; name: string }[];
	activeEditorPath: string | null;
	activeView: string;
	rightBottomCollapsed: boolean;
	rightBottomActiveTab: string;
	reviewCollapsed: boolean;
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
			rightBottomActiveTab: normalizeRightBottomActiveTab(
				internal.rightBottomActiveTab,
			),
			reviewCollapsed: internal.reviewCollapsed,
		},
	};
}
