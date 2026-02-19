import { ChevronDown } from "lucide-react";
import type * as React from "react";
import {
	Collapsible,
	CollapsibleContent,
	CollapsibleTrigger,
} from "@/components/ui/collapsible";
import { cn } from "@/lib/utils";

export interface CollapsibleSectionProps {
	title: string;
	count?: number;
	defaultOpen?: boolean;
	actions?: React.ReactNode;
	className?: string;
	headerClassName?: string;
	chevronClassName?: string;
	children: React.ReactNode;
}

export function CollapsibleSection({
	title,
	count,
	defaultOpen = true,
	actions,
	className,
	headerClassName,
	chevronClassName,
	children,
}: CollapsibleSectionProps) {
	return (
		<Collapsible
			defaultOpen={defaultOpen}
			data-slot="collapsible-section"
			className={cn("overflow-hidden", className)}
		>
			<div
				className={cn(
					"flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium hover:bg-accent/50 transition-colors",
					headerClassName,
				)}
			>
				<CollapsibleTrigger asChild>
					<button
						type="button"
						className="flex flex-1 min-w-0 items-center gap-1"
					>
						<ChevronDown
							className={cn(
								"size-3 shrink-0 transition-transform [[data-state=closed]_&]:-rotate-90",
								chevronClassName,
							)}
						/>
						<span className="flex-1 text-left truncate">
							{title}
							{count != null && ` (${count})`}
						</span>
					</button>
				</CollapsibleTrigger>
				{actions}
			</div>
			<CollapsibleContent>{children}</CollapsibleContent>
		</Collapsible>
	);
}
