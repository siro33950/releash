import { type DragEvent, useCallback, useState } from "react";
import type { SplitDirection } from "@/types/terminal-pane";

const TAB_DRAG_TYPE = "application/x-terminal-tab";
export const PANE_DRAG_TYPE = "application/x-terminal-pane";

type DropZonePosition = "left" | "right" | "top" | "bottom" | "center";

interface PaneDropZoneProps {
	paneId: string;
	children: React.ReactNode;
	onDropTab: (
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
}

function getDropPosition(e: DragEvent, rect: DOMRect): DropZonePosition {
	const x = (e.clientX - rect.left) / rect.width;
	const y = (e.clientY - rect.top) / rect.height;
	const threshold = 0.33;

	if (x < threshold) return "left";
	if (x > 1 - threshold) return "right";
	if (y < threshold) return "top";
	if (y > 1 - threshold) return "bottom";
	return "center";
}

function positionToDirectionAndInsertBefore(
	position: DropZonePosition,
): { direction: SplitDirection; insertBefore: boolean } | null {
	switch (position) {
		case "left":
			return { direction: "vertical", insertBefore: true };
		case "right":
			return { direction: "vertical", insertBefore: false };
		case "top":
			return { direction: "horizontal", insertBefore: true };
		case "bottom":
			return { direction: "horizontal", insertBefore: false };
		default:
			return null;
	}
}

export function PaneDropZone({
	paneId,
	children,
	onDropTab,
	onDropPane,
}: PaneDropZoneProps) {
	const [dropPosition, setDropPosition] = useState<DropZonePosition | null>(
		null,
	);

	const handleDragOver = useCallback(
		(e: DragEvent) => {
			const hasTab = e.dataTransfer.types.includes(TAB_DRAG_TYPE);
			const hasPane =
				Boolean(onDropPane) && e.dataTransfer.types.includes(PANE_DRAG_TYPE);
			if (!hasTab && !hasPane) return;
			e.preventDefault();
			e.dataTransfer.dropEffect = "move";
			const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
			setDropPosition(getDropPosition(e, rect));
		},
		[onDropPane],
	);

	const handleDragLeave = useCallback((e: DragEvent) => {
		if (!e.currentTarget.contains(e.relatedTarget as Node)) {
			setDropPosition(null);
		}
	}, []);

	const handleDrop = useCallback(
		(e: DragEvent) => {
			e.preventDefault();
			if (!dropPosition) {
				setDropPosition(null);
				return;
			}

			// ペインドラッグ優先
			const sourcePaneId = e.dataTransfer.getData(PANE_DRAG_TYPE);
			if (sourcePaneId && onDropPane) {
				const result = positionToDirectionAndInsertBefore(dropPosition);
				if (result) {
					onDropPane(
						sourcePaneId,
						paneId,
						result.direction,
						result.insertBefore,
					);
				}
				setDropPosition(null);
				return;
			}

			// タブドラッグにフォールバック
			const tabId = e.dataTransfer.getData(TAB_DRAG_TYPE);
			if (tabId) {
				const result = positionToDirectionAndInsertBefore(dropPosition);
				if (result) {
					onDropTab(tabId, paneId, result.direction);
				}
			}
			setDropPosition(null);
		},
		[dropPosition, paneId, onDropTab, onDropPane],
	);

	return (
		// biome-ignore lint/a11y/noStaticElementInteractions: DnDドロップゾーン
		<div
			className="relative h-full w-full"
			onDragOver={handleDragOver}
			onDragLeave={handleDragLeave}
			onDrop={handleDrop}
		>
			{children}
			{dropPosition && dropPosition !== "center" && (
				<DropOverlay position={dropPosition} />
			)}
		</div>
	);
}

function DropOverlay({ position }: { position: DropZonePosition }) {
	const style: React.CSSProperties = {
		position: "absolute",
		pointerEvents: "none",
		zIndex: 10,
	};

	switch (position) {
		case "left":
			Object.assign(style, { top: 0, left: 0, bottom: 0, width: "50%" });
			break;
		case "right":
			Object.assign(style, { top: 0, right: 0, bottom: 0, width: "50%" });
			break;
		case "top":
			Object.assign(style, { top: 0, left: 0, right: 0, height: "50%" });
			break;
		case "bottom":
			Object.assign(style, { bottom: 0, left: 0, right: 0, height: "50%" });
			break;
	}

	return (
		<div
			style={style}
			className="bg-primary/15 border-2 border-dashed border-primary/40 rounded"
		/>
	);
}
