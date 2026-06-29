import { cva, type VariantProps } from "class-variance-authority";
import { AlertCircle, ChevronDown, RotateCcw, X } from "lucide-react";
import { useState } from "react";

import { cn } from "@/lib/utils";

const messageVariants = cva("", {
	variants: {
		variant: {
			inline: "flex items-start gap-1 text-xs",
			block: "flex flex-col items-center gap-2 text-sm",
		},
		severity: {
			error: "text-destructive",
			warning: "text-warning",
			info: "text-info",
			success: "text-success",
		},
		size: {
			default: "",
			xs: "text-[10px]",
		},
	},
	defaultVariants: {
		variant: "inline",
		severity: "error",
		size: "default",
	},
});

interface MessageProps extends VariantProps<typeof messageVariants> {
	message: string;
	onDismiss?: () => void;
	onRetry?: () => void;
	expandable?: boolean;
	className?: string;
}

function Message({
	message,
	variant = "inline",
	severity = "error",
	size = "default",
	onDismiss,
	onRetry,
	expandable = false,
	className,
}: MessageProps) {
	const [expanded, setExpanded] = useState(false);

	if (variant === "block") {
		return (
			<div
				data-slot="message"
				className={cn(messageVariants({ variant, severity, size, className }))}
			>
				<AlertCircle className="size-5 shrink-0" />
				<span>{message}</span>
				{onRetry && (
					<button
						type="button"
						className="text-xs text-muted-foreground hover:text-foreground underline"
						onClick={onRetry}
					>
						<RotateCcw className="size-3 inline mr-0.5" />
						Retry
					</button>
				)}
			</div>
		);
	}

	return (
		<div
			data-slot="message"
			className={cn(messageVariants({ variant, severity, size, className }))}
		>
			{expandable && (
				<button
					type="button"
					className="shrink-0 mt-0.5"
					onClick={() => setExpanded((v) => !v)}
				>
					<ChevronDown
						className={cn(
							"size-3 transition-transform",
							expanded && "rotate-180",
						)}
					/>
				</button>
			)}
			<span
				className={cn(
					"flex-1 break-all",
					!expanded && expandable && "truncate",
				)}
			>
				{message}
			</span>
			{onDismiss && (
				<button type="button" className="shrink-0" onClick={onDismiss}>
					<X className="size-3" />
				</button>
			)}
		</div>
	);
}

export { Message };
