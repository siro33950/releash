import {
	closestCenter,
	DndContext,
	type DragEndEvent,
	PointerSensor,
	useSensor,
	useSensors,
} from "@dnd-kit/core";
import {
	horizontalListSortingStrategy,
	SortableContext,
	useSortable,
} from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import type { ComponentProps, ReactNode } from "react";
import { TabsTrigger } from "@/components/ui/tabs";
import { cn } from "@/lib/utils";

interface DraggableTabsProps {
	items: { id: string; draggable?: boolean }[];
	onReorder: (fromIndex: number, toIndex: number) => void;
	children: ReactNode;
}

export function DraggableTabs({
	items,
	onReorder,
	children,
}: DraggableTabsProps) {
	const sensors = useSensors(
		useSensor(PointerSensor, { activationConstraint: { distance: 5 } }),
	);

	const sortableIds = items
		.filter((i) => i.draggable !== false)
		.map((i) => i.id);

	const handleDragEnd = (event: DragEndEvent) => {
		const { active, over } = event;
		if (!over || active.id === over.id) return;
		const fromIndex = items.findIndex((i) => i.id === active.id);
		const toIndex = items.findIndex((i) => i.id === over.id);
		if (fromIndex !== -1 && toIndex !== -1) {
			onReorder(fromIndex, toIndex);
		}
	};

	return (
		<DndContext
			sensors={sensors}
			collisionDetection={closestCenter}
			onDragEnd={handleDragEnd}
		>
			<SortableContext
				items={sortableIds}
				strategy={horizontalListSortingStrategy}
			>
				{children}
			</SortableContext>
		</DndContext>
	);
}

interface SortableTabTriggerProps extends ComponentProps<typeof TabsTrigger> {
	id: string;
	disabled?: boolean;
}

export function SortableTabTrigger({
	id,
	disabled,
	className,
	children,
	style,
	...props
}: SortableTabTriggerProps) {
	const {
		attributes,
		listeners,
		setNodeRef,
		transform,
		transition,
		isDragging,
	} = useSortable({
		id,
		disabled: disabled ? { draggable: true, droppable: true } : false,
	});

	const dragStyle = {
		transform: CSS.Transform.toString(transform),
		transition,
		...style,
	};

	return (
		<TabsTrigger
			disabled={disabled}
			ref={setNodeRef}
			className={cn(isDragging && "opacity-50", className)}
			style={dragStyle}
			{...(disabled ? {} : { ...attributes, ...listeners })}
			{...props}
		>
			{children}
		</TabsTrigger>
	);
}
