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

export function useMcpConfig() {
	const [config, setConfig] = useState<McpConfig>(DEFAULT_CONFIG);
	const [draft, setDraft] = useState<McpConfig>(DEFAULT_CONFIG);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setLoading(true);
		setError(null);
		invoke<McpConfig>("get_mcp_config")
			.then((cfg) => {
				setConfig(cfg);
				setDraft(cfg);
			})
			.catch((e) => {
				setError(String(e));
			})
			.finally(() => {
				setLoading(false);
			});
	}, []);

	const isDirty = JSON.stringify(draft) !== JSON.stringify(config);

	const save = useCallback(async () => {
		setSaving(true);
		setError(null);
		try {
			await invoke("update_mcp_config", {
				port: draft.port,
				token: draft.token,
			});
			setConfig(draft);
		} catch (e) {
			setError(String(e));
			throw e;
		} finally {
			setSaving(false);
		}
	}, [draft]);

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

	const generateConfig = useCallback(async (agentType: McpAgentType) => {
		setError(null);
		try {
			return await invoke<GenerateResult>("generate_agent_mcp_config", {
				agentType,
			});
		} catch (e) {
			setError(String(e));
			throw e;
		}
	}, []);

	const previewConfig = useCallback(async (agentType: McpAgentType) => {
		setError(null);
		try {
			return await invoke<string>("preview_agent_mcp_config", {
				agentType,
			});
		} catch (e) {
			setError(String(e));
			throw e;
		}
	}, []);

	return {
		draft,
		setDraft,
		isDirty,
		loading,
		saving,
		error,
		save,
		regenerateToken,
		generateConfig,
		previewConfig,
	};
}
