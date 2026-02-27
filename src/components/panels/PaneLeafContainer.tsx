import { type DragEvent, useCallback, useRef } from "react";
import { PANE_DRAG_TYPE, PaneDropZone } from "@/components/panels/PaneDropZone";
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
	onFocus: (paneId: string) => void;
	onClose: (paneId: string) => void;
	onSplit: (paneId: string, direction: SplitDirection) => void;
	setTerminalRef: (
		paneId: string,
	) => (handle: TerminalPanelHandle | null) => void;
	onPtyReady?: (paneId: string, ptyId: number, sessionKey: string) => void;
	onDropTab?: (
		tabId: string,
		targetPaneId: string,
		direction: SplitDirection,
	) => void;
	onDropPane?: (
		sourcePaneId: string,
		targetPaneId: string,
		direction: SplitDirection,
		insertBefore: boolean,
	) => void;
	onBreakToTab?: (paneId: string) => void;
	canBreakToTab?: boolean;
}

export function PaneLeafContainer({
	pane,
	isFocused,
	isOnlyPane,
	cwd,
	theme,
	terminalStartupCommand,
	agentType,
	onFocus,
	onClose,
	onSplit,
	setTerminalRef,
	onPtyReady,
	onDropTab,
	onDropPane,
	onBreakToTab,
	canBreakToTab,
}: PaneLeafContainerProps) {
	const localTerminalRef = useRef<TerminalPanelHandle | null>(null);

	const handleFocus = useCallback(() => {
		onFocus(pane.id);
	}, [onFocus, pane.id]);

	const handleClose = useCallback(() => {
		localTerminalRef.current?.requestKill();
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

	const handleDropPane = useCallback(
		(
			sourcePaneId: string,
			targetPaneId: string,
			direction: SplitDirection,
			insertBefore: boolean,
		) => {
			onDropPane?.(sourcePaneId, targetPaneId, direction, insertBefore);
		},
		[onDropPane],
	);

	const handleBreakToTab = useCallback(() => {
		onBreakToTab?.(pane.id);
	}, [onBreakToTab, pane.id]);

	const handlePtyReady = useCallback(
		(ptyId: number, sessionKey: string) => {
			onPtyReady?.(pane.id, ptyId, sessionKey);
		},
		[onPtyReady, pane.id],
	);

	const localSetTerminalRef = useCallback(
		(handle: TerminalPanelHandle | null) => {
			localTerminalRef.current = handle;
			setTerminalRef(pane.id)(handle);
		},
		[setTerminalRef, pane.id],
	);

	const handleDragStart = useCallback(
		(e: DragEvent) => {
			e.dataTransfer.setData(PANE_DRAG_TYPE, pane.id);
			e.dataTransfer.effectAllowed = "move";
		},
		[pane.id],
	);

	const paneHeader = !isOnlyPane && (
		<div className="flex items-center justify-between px-2 py-0.5 bg-muted/30 text-xs text-muted-foreground shrink-0 border-b border-border/50">
			{/* biome-ignore lint/a11y/noStaticElementInteractions: ドラッグハンドル */}
			<span
				className="truncate cursor-grab active:cursor-grabbing"
				draggable
				onDragStart={handleDragStart}
			>
				{pane.label}
			</span>
			<div className="flex items-center gap-1">
				{canBreakToTab && (
					<button
						type="button"
						onClick={handleBreakToTab}
						className="px-1 hover:text-foreground transition-colors"
						aria-label={`${pane.label} をタブに分離`}
						title="タブに分離"
					>
						&#x2934;
					</button>
				)}
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
	);

	const content = (
		// biome-ignore lint/a11y/noStaticElementInteractions: ペインフォーカスにマウスイベントが必要
		<div
			className={`h-full w-full flex flex-col ${
				!isOnlyPane ? "border border-border/50" : ""
			} ${isFocused && !isOnlyPane ? "border-primary/60" : ""}`}
			onMouseDown={handleFocus}
		>
			{paneHeader}
			<div className="flex-1 min-h-0">
				<TerminalPanel
					ref={localSetTerminalRef}
					cwd={cwd}
					theme={theme}
					terminalStartupCommand={terminalStartupCommand}
					agentType={agentType}
					label={pane.label}
					sessionKey={pane.sessionKey ?? undefined}
					onPtyReady={handlePtyReady}
					onSplitVertical={handleSplitVertical}
					onSplitHorizontal={handleSplitHorizontal}
					onBreakToTab={handleBreakToTab}
					onClosePane={handleClose}
					canBreakToTab={canBreakToTab}
					isOnlyPane={isOnlyPane}
				/>
			</div>
		</div>
	);

	if (onDropTab) {
		return (
			<PaneDropZone
				paneId={pane.id}
				onDropTab={handleDropTab}
				onDropPane={onDropPane ? handleDropPane : undefined}
			>
				{content}
			</PaneDropZone>
		);
	}

	return content;
}
