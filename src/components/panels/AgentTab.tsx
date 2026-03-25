import { Bot, Terminal } from "lucide-react";
import { forwardRef, useImperativeHandle, useRef, useState } from "react";
import { AgentChatPanel } from "@/components/panels/AgentChatPanel";
import { EmptyState } from "@/components/panels/EmptyState";
import {
	TerminalTabPanel,
	type TerminalTabPanelHandle,
} from "@/components/panels/TerminalTabPanel";
import { Button } from "@/components/ui/button";
import {
	Tooltip,
	TooltipContent,
	TooltipTrigger,
} from "@/components/ui/tooltip";
import { cn } from "@/lib/utils";
import type { Theme } from "@/types/settings";

type AgentViewMode = "chat" | "terminal";

interface AgentTabProps {
	rootPath: string | null;
	theme?: Theme;
	terminalStartupCommand?: string;
	agentType?: string;
}

export const AgentTab = forwardRef<TerminalTabPanelHandle, AgentTabProps>(
	function AgentTab(
		{ rootPath, theme, terminalStartupCommand, agentType },
		ref,
	) {
		const [viewMode, setViewMode] = useState<AgentViewMode>("chat");
		const terminalRef = useRef<TerminalTabPanelHandle>(null);

		useImperativeHandle(ref, () => ({
			writeToTerminal: (data: string) => {
				terminalRef.current?.writeToTerminal(data);
			},
		}));

		if (!rootPath) {
			return (
				<EmptyState
					title="No worktree selected"
					description="Select a worktree to start an agent session"
				/>
			);
		}

		return (
			<div className="flex flex-col h-full">
				<div className="flex items-center gap-1 px-2 h-8 border-b border-border shrink-0">
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className={cn(
									"h-6 w-6",
									viewMode === "chat"
										? "text-foreground bg-muted"
										: "text-muted-foreground",
								)}
								onClick={() => setViewMode("chat")}
								aria-label="Chat view"
							>
								<Bot className="size-3.5" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom">Chat</TooltipContent>
					</Tooltip>
					<Tooltip>
						<TooltipTrigger asChild>
							<Button
								variant="ghost"
								size="icon"
								className={cn(
									"h-6 w-6",
									viewMode === "terminal"
										? "text-foreground bg-muted"
										: "text-muted-foreground",
								)}
								onClick={() => setViewMode("terminal")}
								aria-label="Terminal view"
							>
								<Terminal className="size-3.5" />
							</Button>
						</TooltipTrigger>
						<TooltipContent side="bottom">Terminal</TooltipContent>
					</Tooltip>
				</div>
				<div className="flex-1 min-h-0 relative">
					<div
						className={cn("absolute inset-0", viewMode !== "chat" && "hidden")}
					>
						<AgentChatPanel worktreePath={rootPath} />
					</div>
					<div
						className={cn(
							"absolute inset-0",
							viewMode !== "terminal" && "invisible h-0 overflow-hidden",
						)}
					>
						<TerminalTabPanel
							ref={terminalRef}
							key={rootPath}
							cwd={rootPath}
							theme={theme}
							terminalStartupCommand={terminalStartupCommand}
							agentType={agentType}
							tabPrefix="Agent"
						/>
					</div>
				</div>
			</div>
		);
	},
);
