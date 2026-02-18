import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";

export interface RemoteConfig {
	auto_start: boolean;
	auto_start_on_lan: boolean;
}

const DEFAULT_CONFIG: RemoteConfig = {
	auto_start: false,
	auto_start_on_lan: false,
};

export function useRemoteConfig() {
	const [config, setConfig] = useState<RemoteConfig>(DEFAULT_CONFIG);
	const [draft, setDraft] = useState<RemoteConfig>(DEFAULT_CONFIG);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setLoading(true);
		setError(null);
		invoke<RemoteConfig>("get_remote_config")
			.then((cfg) => {
				if (cfg) {
					setConfig(cfg);
					setDraft(cfg);
				}
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
			await invoke("update_remote_config", { remote: draft });
			setConfig(draft);
		} catch (e) {
			setError(String(e));
			throw e;
		} finally {
			setSaving(false);
		}
	}, [draft]);

	return { draft, setDraft, isDirty, loading, saving, error, save };
}
