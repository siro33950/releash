import { useCallback } from "react";
import { PaneDropZone } from "@/components/panels/PaneDropZone";
import {
	TerminalPanel,
	type TerminalPanelHandle,
} from "@/components/panels/TerminalPanel";
import type { Theme } from "@/types/settings";
import type { PaneLeaf, SplitDirection } from "@/types/terminal-pane";

interface PaneLeafContainerProps {
	pane: PaneLeaf;
	isFocused: boolean;
	isOnlyPane: boolean;
	cwd?: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;
	agentType?: string;
	sessionKey?: string;
	onFocus: (paneId: string) => void;
	onClose: (paneId: string) => void;
	onSplit: (paneId: string, direction: SplitDirection) => void;
	setTerminalRef: (
		paneId: string,
	) => (handle: TerminalPanelHandle | null) => void;
	onDropTab?: (
		tabId: string,
		targetPaneId: string,
		direction: SplitDirection,
	) => void;
}

export function PaneLeafContainer({
	pane,
	isFocused,
	isOnlyPane,
	cwd,
	theme,
	terminalStartupCommand,
	agentType,
	sessionKey,
	onFocus,
	onClose,
	onSplit,
	setTerminalRef,
	onDropTab,
}: PaneLeafContainerProps) {
	const handleFocus = useCallback(() => {
		onFocus(pane.id);
	}, [onFocus, pane.id]);

	const handleClose = useCallback(() => {
		onClose(pane.id);
	}, [onClose, pane.id]);

	const handleSplitVertical = useCallback(() => {
		onSplit(pane.id, "vertical");
	}, [onSplit, pane.id]);

	const handleSplitHorizontal = useCallback(() => {
		onSplit(pane.id, "horizontal");
	}, [onSplit, pane.id]);

	const handleDropTab = useCallback(
		(tabId: string, targetPaneId: string, direction: SplitDirection) => {
			onDropTab?.(tabId, targetPaneId, direction);
		},
		[onDropTab],
	);

	const content = (
		// biome-ignore lint/a11y/noStaticElementInteractions: ペインフォーカスにマウスイベントが必要
		<div
			className={`h-full w-full flex flex-col ${
				isFocused ? "ring-1 ring-primary/50" : ""
			}`}
			onMouseDown={handleFocus}
		>
			{!isOnlyPane && (
				<div className="flex items-center justify-between px-2 py-0.5 bg-muted/30 text-xs text-muted-foreground shrink-0">
					<span className="truncate">{pane.label}</span>
					<div className="flex items-center gap-1">
						<button
							type="button"
							onClick={handleSplitVertical}
							className="px-1 hover:text-foreground transition-colors"
							aria-label={`${pane.label} を垂直分割`}
							title="垂直分割 (⌘D)"
						>
							┃
						</button>
						<button
							type="button"
							onClick={handleSplitHorizontal}
							className="px-1 hover:text-foreground transition-colors"
							aria-label={`${pane.label} を水平分割`}
							title="水平分割 (⇧⌘D)"
						>
							━
						</button>
						<button
							type="button"
							onClick={handleClose}
							className="px-1 hover:text-foreground transition-colors"
							aria-label={`${pane.label} を閉じる`}
							title="閉じる"
						>
							✕
						</button>
					</div>
				</div>
			)}
			<div className="flex-1 min-h-0">
				<TerminalPanel
					ref={setTerminalRef(pane.id)}
					cwd={cwd}
					theme={theme}
					terminalStartupCommand={terminalStartupCommand}
					agentType={agentType}
					label={pane.label}
					sessionKey={sessionKey ? `${sessionKey}::${pane.id}` : undefined}
				/>
			</div>
		</div>
	);

	if (onDropTab) {
		return (
			<PaneDropZone paneId={pane.id} onDropTab={handleDropTab}>
				{content}
			</PaneDropZone>
		);
	}

	return content;
}
