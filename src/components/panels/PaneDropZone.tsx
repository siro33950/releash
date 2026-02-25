import { type DragEvent, useCallback, useState } from "react";
import type { SplitDirection } from "@/types/terminal-pane";

const DRAG_TYPE = "application/x-terminal-tab";

type DropZonePosition = "left" | "right" | "top" | "bottom" | "center";

interface PaneDropZoneProps {
	paneId: string;
	children: React.ReactNode;
	onDropTab: (
		tabId: string,
		targetPaneId: string,
		direction: SplitDirection,
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

function positionToDirection(
	position: DropZonePosition,
): SplitDirection | null {
	switch (position) {
		case "left":
		case "right":
			return "vertical";
		case "top":
		case "bottom":
			return "horizontal";
		default:
			return null;
	}
}

export function PaneDropZone({
	paneId,
	children,
	onDropTab,
}: PaneDropZoneProps) {
	const [dropPosition, setDropPosition] = useState<DropZonePosition | null>(
		null,
	);

	const handleDragOver = useCallback((e: DragEvent) => {
		if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
		e.preventDefault();
		e.dataTransfer.dropEffect = "move";
		const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
		setDropPosition(getDropPosition(e, rect));
	}, []);

	const handleDragLeave = useCallback((e: DragEvent) => {
		if (!e.currentTarget.contains(e.relatedTarget as Node)) {
			setDropPosition(null);
		}
	}, []);

	const handleDrop = useCallback(
		(e: DragEvent) => {
			e.preventDefault();
			const tabId = e.dataTransfer.getData(DRAG_TYPE);
			if (!tabId || !dropPosition) {
				setDropPosition(null);
				return;
			}
			const direction = positionToDirection(dropPosition);
			if (direction) {
				onDropTab(tabId, paneId, direction);
			}
			setDropPosition(null);
		},
		[dropPosition, paneId, onDropTab],
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
