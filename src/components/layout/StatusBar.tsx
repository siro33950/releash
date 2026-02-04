import { cn } from "@/lib/utils";

interface StatusBarProps {
	className?: string;
	branch?: string;
	status?: string;
}

export function StatusBar({
	className,
	branch = "main",
	status = "Ready",
}: StatusBarProps) {
	return (
		<div
			className={cn(
				"flex items-center justify-between h-6 px-3 select-none",
				"bg-primary text-primary-foreground text-xs",
				className,
			)}
		>
			<div className="flex items-center gap-4">
				<span>{branch}</span>
			</div>
			<div className="flex items-center gap-4">
				<span>{status}</span>
			</div>
		</div>
	);
}
