import { GripVerticalIcon } from "lucide-react";
import type * as React from "react";
import { Group, Panel, Separator } from "react-resizable-panels";

import { cn } from "@/lib/utils";

function ResizablePanelGroup({
	className,
	...props
}: React.ComponentProps<typeof Group>) {
	return <Group className={cn("flex h-full w-full", className)} {...props} />;
}

function ResizablePanel({ ...props }: React.ComponentProps<typeof Panel>) {
	return <Panel {...props} />;
}

function ResizableHandle({
	withHandle,
	className,
	...props
}: React.ComponentProps<typeof Separator> & {
	withHandle?: boolean;
}) {
	return (
		<Separator
			className={cn(
				"relative flex items-center justify-center bg-border",
				"data-[orientation=horizontal]:w-px data-[orientation=horizontal]:cursor-col-resize",
				"data-[orientation=vertical]:h-px data-[orientation=vertical]:cursor-row-resize",
				"hover:bg-primary/50 transition-colors",
				className,
			)}
			{...props}
		>
			{withHandle && (
				<div className="bg-border z-10 flex h-4 w-3 items-center justify-center rounded-sm border">
					<GripVerticalIcon className="size-2.5" />
				</div>
			)}
		</Separator>
	);
}

export { ResizablePanelGroup, ResizablePanel, ResizableHandle };
