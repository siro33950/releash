import { FileText, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

interface EmptyStateProps {
	icon?: LucideIcon;
	title: string;
	description?: string;
	children?: ReactNode;
	compact?: boolean;
	className?: string;
}

export function EmptyState({
	icon: Icon = FileText,
	title,
	description,
	children,
	compact = false,
	className,
}: EmptyStateProps) {
	if (compact) {
		return (
			<div className={cn("px-3 py-2 text-xs text-muted-foreground", className)}>
				{title}
				{children}
			</div>
		);
	}

	return (
		<div
			className={cn(
				"flex flex-col items-center justify-center h-full text-muted-foreground",
				className,
			)}
		>
			<Icon className="h-16 w-16 mb-4 opacity-50" />
			<h3 className="text-lg font-medium mb-2">{title}</h3>
			{description && <p className="text-sm">{description}</p>}
			{children}
		</div>
	);
}
