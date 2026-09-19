import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import { useClientRefresh } from "./useClientRefresh";

export interface BackgroundConfig {
	close_to_tray: boolean;
	auto_launch: boolean;
	start_minimized: boolean;
}

interface LoginItemStatus {
	enabled: boolean;
	requiresApproval: boolean;
	reason: string | null;
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
	const [loginItem, setLoginItem] = useState<LoginItemStatus | null>(null);
	const [cliMessage, setCliMessage] = useState<string | null>(null);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setLoading(true);
		setError(null);

		const settingsRequest = invoke("get_app_settings");
		Promise.all([
			settingsRequest,
			invokeTauri<LoginItemStatus>("get_login_item_status"),
		])
			.then(([settings, loginStatus]) => {
				if (clientRefresh.aborted) return;
				setLoginItem(loginStatus);
				const cfg: BackgroundConfig = {
					close_to_tray: settings.close_to_tray,
					auto_launch: loginStatus.enabled,
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
			if (!loginItem) throw new Error("Background settings are not loaded.");
			let actualLogin = loginItem;
			if (draft.auto_launch !== config.auto_launch) {
				const next = await invokeTauri<LoginItemStatus>(
					"set_login_item_enabled",
					{ enabled: draft.auto_launch },
				);
				actualLogin = next;
				setLoginItem(next);
				setDraft((draft) => ({ ...draft, auto_launch: next.enabled }));
			}

			await invoke("update_app_settings", {
				app: {
					close_to_tray: draft.close_to_tray,
					start_minimized: draft.start_minimized,
				},
			});

			setError(null);
			const saved = {
				...draft,
				auto_launch: actualLogin.enabled,
			};
			setConfig(saved);
			setDraft(saved);
		} catch (e) {
			setError(getErrorMessage(e));
			throw e;
		} finally {
			setSaving(false);
		}
	}, [draft, config, loginItem]);

	const openLoginSettings = async () => {
		try {
			await invokeTauri("open_login_item_settings");
		} catch (error) {
			setError(getErrorMessage(error));
		}
	};
	const installCli = async () => {
		try {
			setCliMessage(await invokeTauri<string>("install_cli"));
			setError(null);
		} catch (error) {
			setError(getErrorMessage(error));
		}
	};
	return {
		loginItem,
		openLoginSettings,
		installCli,
		cliMessage,
		draft,
		setDraft,
		isDirty,
		loading,
		saving,
		error,
		save,
	};
}
