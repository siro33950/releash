export type Theme = "dark" | "light";
export type DiffBase = "branch-base" | "head";
export type DiffSection = "changes" | "staged";
export type DiffMode = "gutter" | "inline" | "split";

export type AgentType =
	| "none"
	| "claude"
	| "codex"
	| "gemini"
	| "aider"
	| "cursor"
	| "custom";

export interface AgentConfig {
	command: string;
	bypassFlag: string;
	label: string;
	modelFlag: string;
}

export const AGENT_MODELS: Record<
	AgentType,
	{ value: string; label: string }[]
> = {
	none: [],
	claude: [
		{ value: "", label: "Default" },
		{ value: "claude-opus-4-6", label: "Opus 4.6" },
		{ value: "claude-sonnet-4-5-20250929", label: "Sonnet 4.5" },
		{ value: "claude-haiku-4-5-20251001", label: "Haiku 4.5" },
	],
	codex: [
		{ value: "", label: "Default" },
		{ value: "gpt-5.3-codex", label: "gpt-5.3-codex" },
		{ value: "gpt-5.3-codex-spark", label: "gpt-5.3-codex-spark" },
	],
	gemini: [
		{ value: "", label: "Default" },
		{ value: "gemini-2.5-pro", label: "Gemini 2.5 Pro" },
		{ value: "gemini-2.5-flash", label: "Gemini 2.5 Flash" },
	],
	aider: [{ value: "", label: "Default" }],
	cursor: [
		{ value: "", label: "Default" },
		{ value: "claude-sonnet-4-6", label: "Sonnet 4.6" },
		{ value: "claude-opus-4-6", label: "Opus 4.6" },
		{ value: "gpt-5.3-codex", label: "gpt-5.3-codex" },
	],
	custom: [],
};

export const AGENT_CONFIGS: Record<AgentType, AgentConfig> = {
	none: {
		command: "",
		bypassFlag: "",
		label: "None",
		modelFlag: "",
	},
	claude: {
		command: "claude",
		bypassFlag: "--dangerously-skip-permissions",
		label: "Claude Code",
		modelFlag: "--model",
	},
	codex: {
		command: "codex",
		bypassFlag: "--dangerously-bypass-approvals-and-sandbox",
		label: "Codex",
		modelFlag: "--model",
	},
	gemini: {
		command: "gemini",
		bypassFlag: "--approval-mode=yolo",
		label: "Gemini CLI",
		modelFlag: "--model",
	},
	aider: {
		command: "aider",
		bypassFlag: "--yes-always",
		label: "Aider",
		modelFlag: "--model",
	},
	cursor: {
		command: "cursor-agent",
		bypassFlag: "",
		label: "Cursor",
		modelFlag: "--model",
	},
	custom: {
		command: "",
		bypassFlag: "",
		label: "Custom",
		modelFlag: "",
	},
};

export interface AppSettings {
	theme: Theme;
	fontSize: number;
	defaultDiffBase: DiffBase;
	defaultDiffMode: DiffMode;
	agent: AgentType;
	agentAutoApprove: boolean;
	terminalStartupCommand: string;
	autoUpdate: boolean;
	telemetryEnabled: boolean;
	enableCrashReporting: boolean;
	defaultDiffOnlyMode: boolean;
	agentMaxConcurrent: number;
}

export const DEFAULT_SETTINGS: AppSettings = {
	theme: "dark",
	fontSize: 14,
	defaultDiffBase: "head",
	defaultDiffMode: "inline",
	defaultDiffOnlyMode: false,
	agent: "none",
	agentAutoApprove: false,
	terminalStartupCommand: "",
	autoUpdate: true,
	telemetryEnabled: true,
	enableCrashReporting: true,
	agentMaxConcurrent: 0,
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
