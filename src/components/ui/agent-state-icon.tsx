import { Bot } from "lucide-react";
import { cn } from "@/lib/utils";
import type { AgentState } from "@/types/protocol";

const colorStyle: Record<AgentState, string> = {
	running: "text-info animate-pulse",
	done: "text-success",
	waiting: "text-warning animate-pulse",
	error: "text-destructive",
};

interface AgentStateIconProps {
	state?: AgentState | null;
	className?: string;
}

export function AgentStateIcon({ state, className }: AgentStateIconProps) {
	const style = state ? colorStyle[state] : "text-muted-foreground";
	return (
		<span title={state ?? undefined}>
			<Bot className={cn("size-3.5 shrink-0", style, className)} />
		</span>
	);
}
