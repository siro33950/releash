import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

export interface McpConfig {
	port: number;
	token: string;
}

export interface GenerateResult {
	file_path: string;
	content: string;
}

export type McpAgentType = "claude" | "codex" | "gemini" | "cursor";

export const MCP_AGENT_OPTIONS: { value: McpAgentType; label: string }[] = [
	{ value: "claude", label: "Claude Code" },
	{ value: "codex", label: "Codex CLI" },
	{ value: "gemini", label: "Gemini CLI" },
	{ value: "cursor", label: "Cursor" },
];

const DEFAULT_CONFIG: McpConfig = {
	port: 19801,
	token: "",
};

function agentSetsEqual(a: McpAgentType[], b: McpAgentType[]): boolean {
	if (a.length !== b.length) return false;
	const sorted = (arr: McpAgentType[]) => [...arr].sort();
	const sa = sorted(a);
	const sb = sorted(b);
	return sa.every((v, i) => v === sb[i]);
}

export function useMcpConfig() {
	const [config, setConfig] = useState<McpConfig>(DEFAULT_CONFIG);
	const [draft, setDraft] = useState<McpConfig>(DEFAULT_CONFIG);
	const [selectedAgents, setSelectedAgents] = useState<McpAgentType[]>([]);
	const [initialAgents, setInitialAgents] = useState<McpAgentType[]>([]);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const [saveResults, setSaveResults] = useState<GenerateResult[]>([]);

	const loadData = useCallback(() => {
		setLoading(true);
		setError(null);
		Promise.all([
			invoke<McpConfig>("get_mcp_config"),
			invoke<string[]>("get_configured_agents"),
		])
			.then(([cfg, agents]) => {
				setConfig(cfg);
				setDraft(cfg);
				const validValues = new Set(MCP_AGENT_OPTIONS.map((o) => o.value));
				const typed = (agents as string[]).filter((a): a is McpAgentType =>
					validValues.has(a as McpAgentType),
				);
				setInitialAgents(typed);
				setSelectedAgents(typed);
			})
			.catch((e) => {
				setError(String(e));
			})
			.finally(() => {
				setLoading(false);
			});
	}, []);

	useEffect(() => {
		loadData();
	}, [loadData]);

	const configDirty = JSON.stringify(draft) !== JSON.stringify(config);
	const agentsDirty = !agentSetsEqual(selectedAgents, initialAgents);
	const isDirty = configDirty || agentsDirty;

	const save = useCallback(async () => {
		setSaving(true);
		setError(null);
		setSaveResults([]);
		try {
			const removed = initialAgents.filter((a) => !selectedAgents.includes(a));
			const results = await invoke<GenerateResult[]>(
				"save_and_generate_mcp_configs",
				{
					port: draft.port,
					token: draft.token,
					agentTypes: selectedAgents,
					removedAgents: removed,
				},
			);
			setConfig(draft);
			setInitialAgents([...selectedAgents]);
			setSaveResults(results);
			return results;
		} catch (e) {
			setError(String(e));
			throw e;
		} finally {
			setSaving(false);
		}
	}, [draft, selectedAgents, initialAgents]);

	const regenerateToken = useCallback(async () => {
		setError(null);
		try {
			const newToken = await invoke<string>("regenerate_mcp_token");
			const updated = { ...draft, token: newToken };
			setDraft(updated);
			setConfig(updated);
		} catch (e) {
			setError(String(e));
		}
	}, [draft]);

	return {
		draft,
		setDraft,
		selectedAgents,
		setSelectedAgents,
		isDirty,
		loading,
		saving,
		error,
		save,
		saveResults,
		regenerateToken,
		reload: loadData,
	};
}
