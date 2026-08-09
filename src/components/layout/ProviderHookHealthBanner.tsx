import { invoke } from "@tauri-apps/api/core";
import { AlertTriangle } from "lucide-react";
import { useEffect, useState } from "react";

interface ProviderHookHealthWarning {
	provider: string;
	launchId: string;
	reason: string;
}

const REFRESH_INTERVAL_MS = 5_000;

function providerLabel(
	provider: ProviderHookHealthWarning["provider"],
): string {
	return `${provider.charAt(0).toUpperCase()}${provider.slice(1)}`;
}

export function ProviderHookHealthBanner() {
	const [warnings, setWarnings] = useState<ProviderHookHealthWarning[]>([]);

	useEffect(() => {
		let active = true;
		const refresh = async () => {
			try {
				const result = await invoke<ProviderHookHealthWarning[]>(
					"list_provider_hook_health_warnings",
				);
				if (active && Array.isArray(result)) setWarnings(result);
			} catch (error) {
				console.warn("Failed to refresh provider hook health warnings:", error);
			}
		};
		void refresh();
		const interval = window.setInterval(
			() => void refresh(),
			REFRESH_INTERVAL_MS,
		);
		return () => {
			active = false;
			window.clearInterval(interval);
		};
	}, []);

	if (warnings.length === 0) return null;
	const providers = [
		...new Set(warnings.map(({ provider }) => providerLabel(provider))),
	];

	return (
		<div
			role="alert"
			className="flex items-center gap-2 border-b border-amber-500/40 bg-amber-500/10 px-3 py-2 text-xs text-amber-900 dark:text-amber-200"
		>
			<AlertTriangle className="size-4 shrink-0" />
			<span>
				Provider Hook health warning: {providers.join(", ")}. AgentSession
				operation remains available.
			</span>
		</div>
	);
}
