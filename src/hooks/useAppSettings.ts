import { invoke } from "@tauri-apps/api/core";
import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useCallback, useEffect, useState } from "react";

export interface BackgroundConfig {
	close_to_tray: boolean;
	auto_launch: boolean;
	start_minimized: boolean;
}

const DEFAULT_CONFIG: BackgroundConfig = {
	close_to_tray: true,
	auto_launch: false,
	start_minimized: false,
};

interface AppSectionResponse {
	close_to_tray: boolean;
	start_minimized: boolean;
	last_root_path: string;
}

export function useBackgroundConfig() {
	const [config, setConfig] = useState<BackgroundConfig>(DEFAULT_CONFIG);
	const [draft, setDraft] = useState<BackgroundConfig>(DEFAULT_CONFIG);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setLoading(true);
		setError(null);

		Promise.all([invoke<AppSectionResponse>("get_app_settings"), isEnabled()])
			.then(([settings, osAutoStartEnabled]) => {
				const cfg: BackgroundConfig = {
					close_to_tray: settings.close_to_tray,
					auto_launch: osAutoStartEnabled,
					start_minimized: settings.start_minimized,
				};
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
			if (draft.auto_launch !== config.auto_launch) {
				if (draft.auto_launch) {
					await enable();
				} else {
					await disable();
				}
			}

			await invoke("update_app_settings", {
				app: {
					close_to_tray: draft.close_to_tray,
					auto_launch: draft.auto_launch,
					start_minimized: draft.start_minimized,
				},
			});

			setConfig({ ...draft });
		} catch (e) {
			setError(String(e));
			throw e;
		} finally {
			setSaving(false);
		}
	}, [draft, config]);

	return { draft, setDraft, isDirty, loading, saving, error, save };
}
