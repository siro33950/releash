import { useEffect, useState } from "react";
import { cn } from "@/lib/utils";
import type { AgentState } from "@/types/protocol";

export function formatElapsed(timestampSec: number): string {
	const now = Date.now() / 1000;
	const diff = Math.max(0, Math.floor(now - timestampSec));
	if (diff < 60) return `${diff}s`;
	if (diff < 3600) return `${Math.floor(diff / 60)}m`;
	return `${Math.floor(diff / 3600)}h`;
}

interface AgentStateConfigEntry {
	bg: string;
	text: string;
	dot: string;
	inlineText: string;
	inlineDot: string;
	label: string;
}

const agentStateConfig: Record<AgentState, AgentStateConfigEntry> = {
	running: {
		bg: "bg-info/15",
		text: "text-info",
		dot: "bg-info animate-pulse",
		inlineText: "text-info",
		inlineDot: "bg-info animate-pulse",
		label: "Running",
	},
	done: {
		bg: "bg-success/15",
		text: "text-success",
		dot: "bg-success",
		inlineText: "text-success",
		inlineDot: "bg-success",
		label: "Done",
	},
	waiting: {
		bg: "bg-warning/15",
		text: "text-warning",
		dot: "bg-warning animate-pulse",
		inlineText: "text-warning",
		inlineDot: "bg-warning animate-pulse",
		label: "Waiting",
	},
	error: {
		bg: "bg-destructive/15",
		text: "text-destructive",
		dot: "bg-destructive",
		inlineText: "text-destructive",
		inlineDot: "bg-destructive",
		label: "Error",
	},
};

interface AgentStateBadgeProps {
	state: AgentState;
	variant?: "badge" | "dot" | "inline";
	timestamp?: number;
	className?: string;
}

export function AgentStateBadge({
	state,
	variant = "badge",
	timestamp,
	className,
}: AgentStateBadgeProps) {
	const [, setTick] = useState(0);
	const config = agentStateConfig[state];

	useEffect(() => {
		if (variant !== "badge" || !timestamp) return;
		const id = setInterval(() => setTick((t) => t + 1), 10000);
		return () => clearInterval(id);
	}, [variant, timestamp]);

	if (variant === "dot") {
		return (
			<span
				className={cn("w-2 h-2 rounded-full shrink-0", config.dot, className)}
				title={state}
			/>
		);
	}

	if (variant === "inline") {
		return (
			<span
				className={cn(
					"inline-flex items-center gap-1",
					config.inlineText,
					className,
				)}
			>
				<span className={cn("w-1.5 h-1.5 rounded-full", config.inlineDot)} />
				Agent: {state}
			</span>
		);
	}

	return (
		<span className={cn("shrink-0 inline-flex items-center gap-1", className)}>
			<span
				className={cn(
					"inline-flex items-center gap-1 text-[10px] px-1.5 py-0.5 rounded font-medium",
					config.bg,
					config.text,
				)}
			>
				<span className={cn("w-1.5 h-1.5 rounded-full", config.dot)} />
				{config.label}
			</span>
			{timestamp && (
				<span className="text-[10px] text-muted-foreground">
					{formatElapsed(timestamp)}
				</span>
			)}
		</span>
	);
}
