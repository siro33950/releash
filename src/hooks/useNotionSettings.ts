import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type {
	NotionPropertyInfo,
	NotionRepoConfig,
	NotionValidationResult,
	PropertyMapping,
} from "@/types/notion";

export interface NotionRepoDraft {
	apiToken: string;
	databaseId: string;
	propertyMapping: PropertyMapping;
	validating: boolean;
	validationStatus: string | null;
	properties: NotionPropertyInfo[];
	markedForDelete: boolean;
}

const EMPTY_MAPPING: PropertyMapping = {
	title: "Name",
	labels: [],
	branch_name: "",
	branch_prefix: "",
};

function configToDraft(config: NotionRepoConfig | null): NotionRepoDraft {
	return {
		apiToken: config?.api_token ?? "",
		databaseId: config?.database_id ?? "",
		propertyMapping: config?.property_mapping ?? { ...EMPTY_MAPPING },
		validating: false,
		validationStatus: null,
		properties: [],
		markedForDelete: false,
	};
}

export interface UseNotionSettingsReturn {
	drafts: Map<string, NotionRepoDraft>;
	loading: boolean;
	isDirty: boolean;
	updateDraft: (
		repoPath: string,
		updater: (d: NotionRepoDraft) => NotionRepoDraft,
	) => void;
	validate: (repoPath: string) => Promise<void>;
	markForDelete: (repoPath: string) => void;
	save: () => Promise<void>;
	reset: () => void;
}

export function useNotionSettings(
	repoPaths: string[],
): UseNotionSettingsReturn {
	const [configs, setConfigs] = useState<Map<string, NotionRepoConfig | null>>(
		new Map(),
	);
	const [drafts, setDrafts] = useState<Map<string, NotionRepoDraft>>(new Map());
	const [loading, setLoading] = useState(true);
	const repoPathsRef = useRef(repoPaths);
	repoPathsRef.current = repoPaths;
	const draftsRef = useRef(drafts);
	draftsRef.current = drafts;
	const configsRef = useRef(configs);
	configsRef.current = configs;
	const loadSeqRef = useRef(0);

	const load = useCallback(async (paths: string[]) => {
		const seq = ++loadSeqRef.current;
		setLoading(true);
		try {
			const entries = await Promise.all(
				paths.map(async (repoPath) => {
					try {
						const config = await invoke<NotionRepoConfig | null>(
							"get_notion_config",
							{ repoPath },
						);
						return [repoPath, config] as const;
					} catch {
						return [repoPath, null] as const;
					}
				}),
			);
			if (seq === loadSeqRef.current) {
				const configMap = new Map(entries);
				const draftMap = new Map(
					entries.map(([path, config]) => [path, configToDraft(config)]),
				);
				setConfigs(configMap);
				setDrafts(draftMap);
			}
		} finally {
			if (seq === loadSeqRef.current) {
				setLoading(false);
			}
		}
	}, []);

	const repoPathsKey = JSON.stringify(repoPaths);

	useEffect(() => {
		const paths = JSON.parse(repoPathsKey) as string[];
		if (paths.length > 0) {
			load(paths);
		} else {
			setConfigs(new Map());
			setDrafts(new Map());
			setLoading(false);
		}
	}, [repoPathsKey, load]);

	const isDirty = (() => {
		for (const [path, draft] of drafts) {
			if (draft.markedForDelete) return true;
			const config = configs.get(path) ?? null;
			const original = configToDraft(config);
			if (
				draft.apiToken !== original.apiToken ||
				draft.databaseId !== original.databaseId ||
				JSON.stringify(draft.propertyMapping) !==
					JSON.stringify(original.propertyMapping)
			) {
				return true;
			}
		}
		return false;
	})();

	const updateDraft = useCallback(
		(repoPath: string, updater: (d: NotionRepoDraft) => NotionRepoDraft) => {
			setDrafts((prev) => {
				const current = prev.get(repoPath);
				if (!current) return prev;
				const next = new Map(prev);
				next.set(repoPath, updater(current));
				return next;
			});
		},
		[],
	);

	const validate = useCallback(
		async (repoPath: string) => {
			const draft = draftsRef.current.get(repoPath);
			if (!draft) return;

			updateDraft(repoPath, (d) => ({
				...d,
				validating: true,
				validationStatus: null,
			}));

			try {
				const result = await invoke<NotionValidationResult>(
					"validate_notion_config",
					{
						apiToken: draft.apiToken,
						databaseId: draft.databaseId,
					},
				);

				let status: string | null = null;
				if (result.status === "configured") {
					status = "success";
				} else if (result.status === "invalid_token") {
					status = "Invalid API token";
				} else if (result.status === "invalid_database") {
					status = "Invalid database ID";
				} else if (result.status === "network_error") {
					status = "Network error: Check your connection";
				} else {
					status = "Configuration incomplete";
				}

				updateDraft(repoPath, (d) => ({
					...d,
					validating: false,
					validationStatus: status,
					properties: result.properties,
				}));
			} catch (e) {
				updateDraft(repoPath, (d) => ({
					...d,
					validating: false,
					validationStatus: String(e),
				}));
			}
		},
		[updateDraft],
	);

	const markForDelete = useCallback(
		(repoPath: string) => {
			updateDraft(repoPath, (d) => ({
				...d,
				markedForDelete: !d.markedForDelete,
			}));
		},
		[updateDraft],
	);

	const save = useCallback(async () => {
		const promises: Promise<void>[] = [];
		const currentDrafts = draftsRef.current;
		const currentConfigs = configsRef.current;

		for (const [path, draft] of currentDrafts) {
			if (draft.markedForDelete) {
				promises.push(invoke("delete_notion_config", { repoPath: path }));
				continue;
			}
			const config = currentConfigs.get(path) ?? null;
			const original = configToDraft(config);
			const changed =
				draft.apiToken !== original.apiToken ||
				draft.databaseId !== original.databaseId ||
				JSON.stringify(draft.propertyMapping) !==
					JSON.stringify(original.propertyMapping);
			if (changed && draft.apiToken && draft.databaseId) {
				promises.push(
					invoke("save_notion_config", {
						repoPath: path,
						apiToken: draft.apiToken,
						databaseId: draft.databaseId,
						propertyMapping: draft.propertyMapping,
					}),
				);
			}
		}

		await Promise.all(promises);
		await load(repoPathsRef.current);
	}, [load]);

	const reset = useCallback(() => {
		const draftMap = new Map<string, NotionRepoDraft>();
		for (const [path, config] of configs) {
			draftMap.set(path, configToDraft(config));
		}
		setDrafts(draftMap);
	}, [configs]);

	return {
		drafts,
		loading,
		isDirty,
		updateDraft,
		validate,
		markForDelete,
		save,
		reset,
	};
}
