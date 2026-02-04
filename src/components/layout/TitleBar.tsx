import { cn } from "@/lib/utils";

interface TitleBarProps {
	className?: string;
}

export function TitleBar({ className }: TitleBarProps) {
	return (
		<div
			className={cn(
				"flex items-center justify-between h-8 px-3 select-none",
				"bg-card border-b border-border",
				className,
			)}
		>
			<div className="flex items-center gap-2">
				<span className="text-muted-foreground text-sm">Releash</span>
			</div>
			<div className="w-20" />
		</div>
	);
}
