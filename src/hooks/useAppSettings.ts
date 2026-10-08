import { invoke as invokeTauri } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import { useStateSubscriptionResult } from "./useStateSubscription";

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
	const desktop = useStateSubscriptionResult("desktop-settings");
	const [config, setConfig] = useState<BackgroundConfig>(DEFAULT_CONFIG);
	const [draft, setDraft] = useState<BackgroundConfig>(DEFAULT_CONFIG);
	const isDirty = JSON.stringify(draft) !== JSON.stringify(config);
	const dirty = useRef(isDirty);
	dirty.current = isDirty;
	const [loginItem, setLoginItem] = useState<LoginItemStatus | null>(null);
	const [loginError, setLoginError] = useState<string | null>(null);
	const [cliMessage, setCliMessage] = useState<string | null>(null);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		let active = true;
		invokeTauri<LoginItemStatus>("get_login_item_status")
			.then((status) => {
				if (active) setLoginItem(status);
			})
			.catch((e) => {
				if (active) setLoginError(getErrorMessage(e));
			});
		return () => {
			active = false;
		};
	}, []);

	const settings = desktop.value;
	useEffect(() => {
		if (!settings || !loginItem || dirty.current) return;
		const cfg: BackgroundConfig = {
			close_to_tray: settings.closeToTray,
			auto_launch: loginItem.enabled,
			start_minimized: settings.startMinimized,
		};
		setConfig(cfg);
		setDraft(cfg);
	}, [settings, loginItem]);

	const loading = !settings || !loginItem;
	const loadError = desktop.error ?? loginError;

	const save = useCallback(async () => {
		setSaving(true);
		setError(null);
		try {
			if (!loginItem || !settings)
				throw new Error("Background settings are not loaded.");
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
	}, [draft, config, loginItem, settings]);

	const openLoginSettings = async () => {
		try {
			await invokeTauri("open_login_item_settings");
		} catch (error) {
			setError(getErrorMessage(error));
		}
	};
	const installCli = async () => {
		try {
			setCliMessage(null);
			const result = await invoke("install_cli");
			setCliMessage(
				`Releash CLI ${result.status === "alreadyInstalled" ? "already installed" : "installed"} at ${result.path}`,
			);
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
		loading: loading && !loadError,
		saving,
		error: error ?? loadError,
		save,
	};
}
