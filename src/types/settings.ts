export type Theme = "dark" | "light";
export type DiffBase = "HEAD" | "staged";
export type DiffMode = "gutter" | "inline" | "split";

export type AgentType = "none" | "claude" | "codex" | "gemini" | "aider" | "cursor" | "custom";

export interface AgentConfig {
	command: string;
	bypassFlag: string;
	label: string;
}

export const AGENT_CONFIGS: Record<AgentType, AgentConfig> = {
	none: { command: "", bypassFlag: "", label: "None" },
	claude: { command: "claude", bypassFlag: "--dangerously-skip-permissions", label: "Claude Code" },
	codex: { command: "codex", bypassFlag: "--yolo", label: "Codex" },
	gemini: { command: "gemini", bypassFlag: "--yolo", label: "Gemini CLI" },
	aider: { command: "aider", bypassFlag: "--yes-always", label: "Aider" },
	cursor: { command: "cursor-agent", bypassFlag: "", label: "Cursor" },
	custom: { command: "", bypassFlag: "", label: "Custom" },
};

export interface AppSettings {
	theme: Theme;
	fontSize: number;
	defaultDiffBase: DiffBase;
	defaultDiffMode: DiffMode;
	agent: AgentType;
	agentAutoApprove: boolean;
	terminalStartupCommand: string;
}

export const DEFAULT_SETTINGS: AppSettings = {
	theme: "dark",
	fontSize: 14,
	defaultDiffBase: "staged",
	defaultDiffMode: "inline",
	agent: "none",
	agentAutoApprove: false,
	terminalStartupCommand: "",
};

export function buildTerminalCommand(settings: AppSettings): string {
	const { agent, agentAutoApprove, terminalStartupCommand } = settings;

	if (agent === "none") {
		return "";
	}

	if (agent === "custom") {
		return terminalStartupCommand;
	}

	const config = AGENT_CONFIGS[agent];
	let agentCmd = config.command;
	if (agentAutoApprove && config.bypassFlag) {
		agentCmd += ` ${config.bypassFlag}`;
	}

	return agentCmd;
}
