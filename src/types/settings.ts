export type Theme = "dark" | "light";
export type DiffBase = "branch-base" | "staged";
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
	reviewCommand: string | null;
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
		reviewCommand: null,
		modelFlag: "",
	},
	claude: {
		command: "claude",
		bypassFlag: "--dangerously-skip-permissions",
		label: "Claude Code",
		reviewCommand:
			'echo "{prompt}" | claude -p --verbose --output-format stream-json --permission-mode bypassPermissions --allowedTools "Read,Bash,Glob,Grep,mcp__releash__worktrees_list,mcp__releash__post_review_comment,mcp__releash__get_review_comments,mcp__releash__resolve_comment,mcp__releash__review_diff,mcp__releash__read_file,mcp__releash__check_diagnostics,mcp__releash__get_file_symbols,mcp__releash__explore_symbol" {model_flag}',
		modelFlag: "--model",
	},
	codex: {
		command: "codex",
		bypassFlag: "--dangerously-bypass-approvals-and-sandbox",
		label: "Codex",
		reviewCommand:
			'codex exec --sandbox read-only --ask-for-approval never --json {model_flag} "{prompt}"',
		modelFlag: "--model",
	},
	gemini: {
		command: "gemini",
		bypassFlag: "--approval-mode=yolo",
		label: "Gemini CLI",
		reviewCommand:
			'gemini -p --sandbox --output-format json {model_flag} "{prompt}"',
		modelFlag: "--model",
	},
	aider: {
		command: "aider",
		bypassFlag: "--yes-always",
		label: "Aider",
		reviewCommand: 'aider --message --yes-always {model_flag} "{prompt}"',
		modelFlag: "--model",
	},
	cursor: {
		command: "cursor-agent",
		bypassFlag: "",
		label: "Cursor",
		reviewCommand:
			'cursor-agent -p --output-format json {model_flag} "{prompt}"',
		modelFlag: "--model",
	},
	custom: {
		command: "",
		bypassFlag: "",
		label: "Custom",
		reviewCommand: null,
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
	reviewAgent: AgentType;
	reviewModel: string;
	customReviewCommand: string;
	autoUpdate: boolean;
	telemetryEnabled: boolean;
	enableCrashReporting: boolean;
}

export const DEFAULT_SETTINGS: AppSettings = {
	theme: "dark",
	fontSize: 14,
	defaultDiffBase: "staged",
	defaultDiffMode: "inline",
	agent: "none",
	agentAutoApprove: false,
	terminalStartupCommand: "",
	reviewAgent: "claude",
	reviewModel: "",
	customReviewCommand: "",
	autoUpdate: true,
	telemetryEnabled: true,
	enableCrashReporting: true,
};

export function buildReviewCommand(
	settings: AppSettings,
	prompt: string,
): string | null {
	const { reviewAgent, reviewModel, customReviewCommand } = settings;

	if (reviewAgent === "none") {
		return null;
	}

	const escapedPrompt = prompt
		.replace(/\\/g, "\\\\")
		.replace(/"/g, '\\"')
		.replace(/\$/g, "\\$")
		.replace(/`/g, "\\`");

	if (reviewAgent === "custom") {
		if (!customReviewCommand) return null;
		return customReviewCommand.replace("{prompt}", escapedPrompt);
	}

	const config = AGENT_CONFIGS[reviewAgent];
	if (!config.reviewCommand) return null;

	const allowedModels = AGENT_MODELS[reviewAgent].map((m) => m.value);
	const safeModel =
		reviewModel && allowedModels.includes(reviewModel) ? reviewModel : "";
	const modelFlagValue =
		safeModel && config.modelFlag ? `${config.modelFlag} ${safeModel}` : "";

	return config.reviewCommand
		.replace("{model_flag}", modelFlagValue)
		.replace(/\s{2,}/g, " ")
		.replace("{prompt}", escapedPrompt)
		.trim();
}

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
