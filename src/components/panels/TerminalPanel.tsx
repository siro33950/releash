import { Terminal } from "lucide-react";
import { cn } from "@/lib/utils";

export function TerminalPanel() {
	return (
		<div
			className={cn(
				"h-full p-3 font-mono text-sm",
				"bg-terminal-bg text-terminal-foreground",
			)}
		>
			<div className="flex items-center gap-2 mb-3 text-muted-foreground">
				<Terminal className="size-4" />
				<span className="text-xs">xterm.js will be integrated here</span>
			</div>
			<div className="flex items-center">
				<span className="text-terminal-blue">user@releash</span>
				<span className="text-terminal-foreground">:</span>
				<span className="text-terminal-green">~</span>
				<span className="text-terminal-foreground">$ </span>
				<span className="animate-pulse">_</span>
			</div>
		</div>
	);
}
