import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type {
	NotionRepoConfig,
	NotionValidationResult,
	PropertyMapping,
} from "@/types/notion";

export function useNotionConfig(repoPath: string) {
	const [config, setConfig] = useState<NotionRepoConfig | null>(null);
	const [loading, setLoading] = useState(true);

	const load = useCallback(async () => {
		setLoading(true);
		try {
			const result = await invoke<NotionRepoConfig | null>(
				"get_notion_config",
				{ repoPath },
			);
			setConfig(result);
		} catch {
			setConfig(null);
		} finally {
			setLoading(false);
		}
	}, [repoPath]);

	useEffect(() => {
		load();
	}, [load]);

	const save = useCallback(
		async (
			apiToken: string,
			databaseId: string,
			propertyMapping: PropertyMapping,
		) => {
			await invoke("save_notion_config", {
				repoPath,
				apiToken,
				databaseId,
				propertyMapping,
			});
			await load();
		},
		[repoPath, load],
	);

	const remove = useCallback(async () => {
		await invoke("delete_notion_config", { repoPath });
		setConfig(null);
	}, [repoPath]);

	const validate = useCallback(async (apiToken: string, databaseId: string) => {
		return invoke<NotionValidationResult>("validate_notion_config", {
			apiToken,
			databaseId,
		});
	}, []);

	const isConfigured = config !== null;

	return { config, loading, save, remove, validate, isConfigured };
}
