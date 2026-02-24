import { forwardRef } from "react";
import { EmptyState } from "@/components/panels/EmptyState";
import {
	TerminalTabPanel,
	type TerminalTabPanelHandle,
} from "@/components/panels/TerminalTabPanel";
import type { Theme } from "@/types/settings";

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
		if (!rootPath) {
			return (
				<EmptyState
					title="No worktree selected"
					description="Select a worktree to start an agent session"
				/>
			);
		}

		return (
			<TerminalTabPanel
				ref={ref}
				key={rootPath}
				cwd={rootPath}
				theme={theme}
				terminalStartupCommand={terminalStartupCommand}
				agentType={agentType}
				sessionKey={`agent::${rootPath}`}
			/>
		);
	},
);
