import { disable, enable, isEnabled } from "@tauri-apps/plugin-autostart";
import { useCallback, useEffect, useRef, useState } from "react";
import {
	type ClientTransportError,
	invokeClient as invoke,
} from "@/lib/clientSocket";
import { getErrorMessage } from "@/lib/errorMessage";
import { useClientRefresh } from "./useClientRefresh";

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

export function useBackgroundConfig() {
	const clientRefresh = useClientRefresh();
	const [config, setConfig] = useState<BackgroundConfig>(DEFAULT_CONFIG);
	const [draft, setDraft] = useState<BackgroundConfig>(DEFAULT_CONFIG);
	const isDirty = JSON.stringify(draft) !== JSON.stringify(config);
	const dirty = useRef(isDirty);
	dirty.current = isDirty;
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [uncertain, setUncertain] = useState<ClientTransportError | null>(null);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setLoading(true);
		setError(null);

		const settingsRequest = invoke("get_app_settings");
		Promise.all([settingsRequest, isEnabled()])
			.then(([settings, osAutoStartEnabled]) => {
				if (clientRefresh.aborted) return;
				const cfg: BackgroundConfig = {
					close_to_tray: settings.close_to_tray,
					auto_launch: osAutoStartEnabled,
					start_minimized: settings.start_minimized,
				};
				if (!dirty.current) {
					setConfig(cfg);
					setDraft(cfg);
				}
			})
			.catch((e) => {
				if (!clientRefresh.aborted) setError(getErrorMessage(e));
			})
			.finally(() => {
				if (!clientRefresh.aborted) setLoading(false);
			});
	}, [clientRefresh]);

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

			await invoke(
				"update_app_settings",
				{
					app: {
						close_to_tray: draft.close_to_tray,
						auto_launch: draft.auto_launch,
						start_minimized: draft.start_minimized,
					},
				},
				{ onUncertain: setUncertain },
			);

			setError(null);
			setConfig({ ...draft });
		} catch (e) {
			setError(getErrorMessage(e));
			throw e;
		} finally {
			setSaving(false);
			setUncertain(null);
		}
	}, [draft, config]);

	return {
		draft,
		setDraft,
		isDirty,
		loading,
		saving,
		uncertain,
		error: uncertain?.message ?? error,
		save,
	};
}
