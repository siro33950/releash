import { cva, type VariantProps } from "class-variance-authority";
import { CircleAlert, CircleCheck, Info, TriangleAlert, X } from "lucide-react";
import type * as React from "react";

import { cn } from "@/lib/utils";

const iconMap = {
	error: CircleAlert,
	warning: TriangleAlert,
	info: Info,
	success: CircleCheck,
} as const;

const inlineMessageVariants = cva("", {
	variants: {
		type: {
			error: "text-destructive",
			warning: "text-warning",
			info: "text-info",
			success: "text-success",
		},
		size: {
			xs: "text-[10px]",
			sm: "text-xs",
			default: "text-sm",
		},
		filled: {
			true: "rounded px-2 py-1.5",
			false: "",
		},
		layout: {
			inline: "flex items-start gap-1",
			block: "flex flex-col items-center gap-2",
		},
	},
	compoundVariants: [
		{ filled: true, type: "error", className: "bg-destructive/10" },
		{
			filled: true,
			type: "warning",
			className: "bg-warning/10 border border-warning/30",
		},
		{ filled: true, type: "info", className: "bg-info/10" },
		{ filled: true, type: "success", className: "bg-success/10" },
	],
	defaultVariants: {
		type: "error",
		size: "sm",
		filled: false,
		layout: "inline",
	},
});

interface InlineMessageProps
	extends React.HTMLAttributes<HTMLDivElement>,
		VariantProps<typeof inlineMessageVariants> {
	icon?: boolean;
	onDismiss?: () => void;
	onRetry?: () => void;
}

function InlineMessage({
	className,
	type = "error",
	size = "sm",
	filled = false,
	layout = "inline",
	icon = false,
	onDismiss,
	onRetry,
	children,
	...props
}: InlineMessageProps) {
	const Icon = type ? iconMap[type] : null;
	const iconSize =
		size === "xs" ? "size-2.5" : size === "sm" ? "size-3" : "size-3.5";

	if (layout === "block") {
		return (
			<div
				data-slot="inline-message"
				className={cn(
					inlineMessageVariants({ type, size, filled, layout, className }),
				)}
				{...props}
			>
				{icon && Icon && <Icon className={iconSize} />}
				<span>{children}</span>
				{onRetry && (
					<button
						type="button"
						className="text-xs text-muted-foreground hover:text-foreground underline"
						onClick={onRetry}
					>
						Retry
					</button>
				)}
			</div>
		);
	}

	return (
		<div
			data-slot="inline-message"
			className={cn(
				inlineMessageVariants({ type, size, filled, layout, className }),
			)}
			{...props}
		>
			{icon && Icon && <Icon className={cn(iconSize, "shrink-0 mt-0.5")} />}
			<span className="flex-1 break-all">{children}</span>
			{onRetry && (
				<button
					type="button"
					className="shrink-0 underline hover:no-underline"
					onClick={onRetry}
				>
					Retry
				</button>
			)}
			{onDismiss && (
				<button
					type="button"
					className="shrink-0"
					onClick={onDismiss}
					aria-label="Dismiss"
				>
					<X className={iconSize} />
				</button>
			)}
		</div>
	);
}

export { InlineMessage, inlineMessageVariants };
