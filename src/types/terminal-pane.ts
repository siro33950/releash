export type SplitDirection = "horizontal" | "vertical";

export interface PaneLeaf {
	type: "leaf";
	id: string;
	label: string;
	ptyId: number | null;
	sessionKey: string | null;
	pendingKill?: boolean;
}

export interface PaneContainer {
	type: "container";
	id: string;
	direction: SplitDirection;
	children: PaneNode[];
	ratios: number[];
}

export type PaneNode = PaneLeaf | PaneContainer;

export interface TerminalTab {
	id: string;
	label: string;
	paneTree: PaneNode;
	focusedPaneId: string;
}
