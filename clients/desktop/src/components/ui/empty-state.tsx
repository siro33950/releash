import { FileText, type LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

interface EmptyStateAction {
	label: string;
	onClick: () => void;
	icon?: LucideIcon;
}

interface EmptyStateProps {
	icon?: LucideIcon;
	title: string;
	description?: ReactNode;
	children?: ReactNode;
	compact?: boolean;
	className?: string;
	action?: EmptyStateAction;
}

export function EmptyState({
	icon: Icon = FileText,
	title,
	description,
	children,
	compact = false,
	className,
	action,
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
				"flex flex-col items-center justify-center h-full gap-2 text-muted-foreground px-4",
				className,
			)}
		>
			<Icon className="h-8 w-8" />
			<span className="text-xs font-medium">{title}</span>
			{description && (
				<div className="text-[11px] text-center leading-relaxed">
					{description}
				</div>
			)}
			{action && (
				<Button variant="outline" size="xs" onClick={action.onClick}>
					{action.icon && <action.icon className="size-3" />}
					{action.label}
				</Button>
			)}
			{children}
		</div>
	);
}
