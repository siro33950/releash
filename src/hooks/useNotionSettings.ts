import { useCallback, useEffect, useRef, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import type {
	NotionPropertyInfo,
	NotionRepoConfig,
	PropertyMapping,
} from "@/types/notion";
import { useClientRefresh } from "./useClientRefresh";

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

function draftChanged(draft: NotionRepoDraft, config: NotionRepoConfig | null) {
	const original = configToDraft(config);
	return (
		draft.markedForDelete ||
		draft.apiToken !== original.apiToken ||
		draft.databaseId !== original.databaseId ||
		JSON.stringify(draft.propertyMapping) !==
			JSON.stringify(original.propertyMapping)
	);
}

export interface UseNotionSettingsReturn {
	drafts: Map<string, NotionRepoDraft>;
	errors: Map<string, string>;
	saveError: string | null;
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
	const clientRefresh = useClientRefresh();
	const [saveError, setSaveError] = useState<string | null>(null);
	const [configs, setConfigs] = useState<Map<string, NotionRepoConfig | null>>(
		new Map(),
	);
	const [drafts, setDrafts] = useState<Map<string, NotionRepoDraft>>(new Map());
	const [errors, setErrors] = useState<Map<string, string>>(new Map());
	const [loading, setLoading] = useState(true);
	const repoPathsRef = useRef(repoPaths);
	repoPathsRef.current = repoPaths;
	const draftsRef = useRef(drafts);
	draftsRef.current = drafts;
	const configsRef = useRef(configs);
	configsRef.current = configs;
	const loadSeqRef = useRef(0);

	const load = useCallback(async (paths: string[], refresh: AbortSignal) => {
		if (refresh.aborted) return;
		const seq = ++loadSeqRef.current;
		setLoading(true);
		try {
			const results = await Promise.allSettled(
				paths.map((repoPath) => invoke("get_notion_config", { repoPath })),
			);
			if (seq === loadSeqRef.current && !refresh.aborted) {
				const configMap = new Map<string, NotionRepoConfig | null>();
				const draftMap = new Map<string, NotionRepoDraft>();
				const errorMap = new Map<string, string>();
				results.forEach((result, index) => {
					const path = paths[index];
					const draft = draftsRef.current.get(path);
					const config = configsRef.current.get(path) ?? null;
					if (draft && draftChanged(draft, config)) {
						configMap.set(path, config);
						draftMap.set(path, draft);
						if (result.status === "rejected")
							errorMap.set(path, getErrorMessage(result.reason));
					} else if (result.status === "fulfilled") {
						configMap.set(path, result.value);
						draftMap.set(path, configToDraft(result.value));
					} else {
						errorMap.set(path, getErrorMessage(result.reason));
					}
				});
				setConfigs(configMap);
				setDrafts(draftMap);
				setErrors(errorMap);
			}
		} finally {
			if (seq === loadSeqRef.current && !refresh.aborted) {
				setLoading(false);
			}
		}
	}, []);

	const repoPathsKey = JSON.stringify(repoPaths);

	useEffect(() => {
		const paths = JSON.parse(repoPathsKey) as string[];
		if (paths.length > 0) {
			load(paths, clientRefresh);
		} else {
			setConfigs(new Map());
			setDrafts(new Map());
			setErrors(new Map());
			setLoading(false);
		}
	}, [repoPathsKey, load, clientRefresh]);

	const isDirty = [...drafts].some(([path, draft]) =>
		draftChanged(draft, configs.get(path) ?? null),
	);

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
				const result = await invoke("validate_notion_config", {
					apiToken: draft.apiToken,
					databaseId: draft.databaseId,
				});

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
					validationStatus: getErrorMessage(e),
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
		setSaveError(null);
		const promises: Promise<void>[] = [];
		const currentDrafts = draftsRef.current;
		const currentConfigs = configsRef.current;

		for (const [path, draft] of currentDrafts) {
			if (draft.markedForDelete) {
				promises.push(
					invoke("delete_notion_config", { repoPath: path }).then(() => {
						configsRef.current = new Map(configsRef.current).set(path, null);
						setConfigs(configsRef.current);
						if (draftsRef.current.get(path) === draft) {
							draftsRef.current = new Map(draftsRef.current).set(
								path,
								configToDraft(null),
							);
							setDrafts(draftsRef.current);
						}
					}),
				);
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
					}).then(() => {
						const config = {
							api_token: draft.apiToken,
							database_id: draft.databaseId,
							property_mapping: draft.propertyMapping,
						};
						configsRef.current = new Map(configsRef.current).set(path, config);
						setConfigs(configsRef.current);
					}),
				);
			}
		}

		try {
			await Promise.all(promises);
		} catch (error) {
			setSaveError(getErrorMessage(error));
			throw error;
		} finally {
			await load(repoPathsRef.current, clientRefresh);
		}
	}, [load, clientRefresh]);

	const reset = useCallback(() => {
		const draftMap = new Map<string, NotionRepoDraft>();
		for (const [path, config] of configs) {
			draftMap.set(path, configToDraft(config));
		}
		setDrafts(draftMap);
	}, [configs]);

	return {
		drafts,
		errors,
		saveError,
		loading,
		isDirty,
		updateDraft,
		validate,
		markForDelete,
		save,
		reset,
	};
}
