import { type DragEvent, useCallback, useRef, useState } from "react";

const DRAG_TYPE = "application/x-workspace-tab";

export interface TabDragItem {
	tabId: string;
	isDraggable: boolean;
}

export type DropPosition = "left" | "right" | null;

export interface UseTabDragReturn {
	dragHandlers: (item: TabDragItem) => {
		draggable: boolean;
		onDragStart: (e: DragEvent) => void;
		onDragEnd: (e: DragEvent) => void;
		onDragOver: (e: DragEvent) => void;
		onDragLeave: (e: DragEvent) => void;
		onDrop: (e: DragEvent) => void;
	};
	draggingId: string | null;
	dropTarget: { tabId: string; position: DropPosition } | null;
}

export function useTabDrag(
	onReorder: (fromId: string, toId: string) => void,
): UseTabDragReturn {
	const [draggingId, setDraggingId] = useState<string | null>(null);
	const [dropTarget, setDropTarget] = useState<{
		tabId: string;
		position: DropPosition;
	} | null>(null);
	const draggingIdRef = useRef<string | null>(null);

	const dragHandlers = useCallback(
		(item: TabDragItem) => ({
			draggable: item.isDraggable,
			onDragStart: (e: DragEvent) => {
				if (!item.isDraggable) {
					e.preventDefault();
					return;
				}
				e.dataTransfer.setData(DRAG_TYPE, item.tabId);
				e.dataTransfer.effectAllowed = "move";
				draggingIdRef.current = item.tabId;
				setDraggingId(item.tabId);
			},
			onDragEnd: (_e: DragEvent) => {
				draggingIdRef.current = null;
				setDraggingId(null);
				setDropTarget(null);
			},
			onDragOver: (e: DragEvent) => {
				if (!e.dataTransfer.types.includes(DRAG_TYPE)) return;
				e.preventDefault();
				e.dataTransfer.dropEffect = "move";
				const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
				const midX = rect.left + rect.width / 2;
				const position: DropPosition = e.clientX < midX ? "left" : "right";
				setDropTarget({ tabId: item.tabId, position });
			},
			onDragLeave: (_e: DragEvent) => {
				setDropTarget((prev) => (prev?.tabId === item.tabId ? null : prev));
			},
			onDrop: (e: DragEvent) => {
				e.preventDefault();
				const fromId = e.dataTransfer.getData(DRAG_TYPE);
				if (fromId && fromId !== item.tabId) {
					onReorder(fromId, item.tabId);
				}
				setDropTarget(null);
				draggingIdRef.current = null;
				setDraggingId(null);
			},
		}),
		[onReorder],
	);

	return { dragHandlers, draggingId, dropTarget };
}
