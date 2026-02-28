import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

export interface SkillMcpConfig {
	tools: string[];
}

export interface SkillDefinition {
	name: string;
	description?: string;
	agent: string;
	model?: string;
	command: string;
	prompt_template: string;
	timeout?: number;
	mcp_config?: SkillMcpConfig;
}

export function useSkills(repoPath: string | null) {
	const [skills, setSkills] = useState<SkillDefinition[]>([]);
	const [loading, setLoading] = useState(false);

	const refresh = useCallback(async () => {
		if (!repoPath) {
			setSkills([]);
			return;
		}
		setLoading(true);
		try {
			const result = await invoke<SkillDefinition[]>("list_skills", {
				repoPath,
			});
			setSkills(result);
		} catch {
			setSkills([]);
		} finally {
			setLoading(false);
		}
	}, [repoPath]);

	useEffect(() => {
		refresh();
	}, [refresh]);

	return { skills, loading, refresh };
}
