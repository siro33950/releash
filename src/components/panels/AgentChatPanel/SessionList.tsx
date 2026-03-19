import { MessageSquare, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";
import type { SessionSummary } from "@/types/session";

interface SessionListProps {
	sessions: SessionSummary[];
	activeSessionId: string | null;
	onSelect: (sessionId: string) => void;
	onNew: () => void;
}

function formatTime(timestamp: number): string {
	const date = new Date(timestamp * 1000);
	const now = new Date();
	const isToday = date.toDateString() === now.toDateString();
	if (isToday) {
		return date.toLocaleTimeString(undefined, {
			hour: "2-digit",
			minute: "2-digit",
		});
	}
	return date.toLocaleDateString(undefined, {
		month: "short",
		day: "numeric",
	});
}

export function SessionList({
	sessions,
	activeSessionId,
	onSelect,
	onNew,
}: SessionListProps) {
	return (
		<div
			data-testid="session-list"
			className="flex flex-col h-full border-r border-border w-56 shrink-0"
		>
			<div className="flex items-center justify-between px-3 py-2 border-b border-border">
				<span className="text-xs font-medium text-muted-foreground uppercase">
					Sessions
				</span>
				<Button
					variant="ghost"
					size="icon"
					className="h-6 w-6"
					onClick={onNew}
					aria-label="New session"
				>
					<Plus className="size-3.5" />
				</Button>
			</div>
			<div className="flex-1 overflow-y-auto">
				{sessions.map((session) => (
					<button
						type="button"
						key={session.id}
						onClick={() => onSelect(session.id)}
						className={cn(
							"w-full text-left px-3 py-2 text-sm hover:bg-muted/50 transition-colors border-b border-border/50",
							session.id === activeSessionId && "bg-muted",
						)}
					>
						<div className="flex items-center gap-2">
							<MessageSquare className="size-3.5 text-muted-foreground shrink-0" />
							<span className="truncate flex-1">
								{session.firstMessage || "New session"}
							</span>
						</div>
						<div className="flex items-center justify-between mt-1">
							<span className="text-xs text-muted-foreground">
								{session.messageCount} messages
							</span>
							<span className="text-xs text-muted-foreground">
								{formatTime(session.updatedAt)}
							</span>
						</div>
					</button>
				))}
				{sessions.length === 0 && (
					<div className="px-3 py-4 text-xs text-muted-foreground text-center">
						No sessions yet
					</div>
				)}
			</div>
		</div>
	);
}
