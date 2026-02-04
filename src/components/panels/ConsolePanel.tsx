import { CircleAlert, CircleCheck, Info } from "lucide-react";
import { cn } from "@/lib/utils";

export function ConsolePanel() {
	return (
		<div className={cn("h-full p-3 font-mono text-xs", "bg-card")}>
			<div className="flex items-center gap-2 mb-1 text-terminal-blue">
				<Info className="size-3" />
				<span>Console output panel</span>
			</div>
			<div className="flex items-center gap-2 mb-1 text-terminal-yellow">
				<CircleAlert className="size-3" />
				<span>This is a placeholder for build/task output</span>
			</div>
			<div className="flex items-center gap-2 text-terminal-green">
				<CircleCheck className="size-3" />
				<span>Ready</span>
			</div>
		</div>
	);
}
