import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useState } from "react";
import type { NotifyConfig } from "@/types/webhook";

const DEFAULT_CONFIG: NotifyConfig = {
	webhook_url: "",
	on_running: false,
	on_done: true,
	on_error: true,
	on_waiting: true,
	desktop_mode: "always",
	inactive_timeout_minutes: 2,
};

export function useWebhookConfig() {
	const [config, setConfig] = useState<NotifyConfig>(DEFAULT_CONFIG);
	const [draft, setDraft] = useState<NotifyConfig>(DEFAULT_CONFIG);
	const [loading, setLoading] = useState(true);
	const [saving, setSaving] = useState(false);
	const [error, setError] = useState<string | null>(null);

	useEffect(() => {
		setLoading(true);
		setError(null);
		invoke<NotifyConfig>("get_notify_config")
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
			await invoke("update_notify_config", { notify: draft });
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
