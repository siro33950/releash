import { listen } from "@tauri-apps/api/event";

const PROVIDER_AGENT_SESSION_CHANGED = "provider-agent-session-refresh";
const PROVIDER_AGENT_SESSION_CHANGED_BACKEND = "provider-agent-session-changed";

interface ProviderAgentSessionChangedDetail {
	worktreePath?: string;
}

export function notifyProviderAgentSessionChanged(worktreePath: string) {
	window.dispatchEvent(
		new CustomEvent<ProviderAgentSessionChangedDetail>(
			PROVIDER_AGENT_SESSION_CHANGED,
			{ detail: { worktreePath } },
		),
	);
}

export function subscribeProviderAgentSessionChanged(
	listener: (detail: ProviderAgentSessionChangedDetail) => void,
) {
	const handleEvent = (event: Event) => {
		listener(
			(event as CustomEvent<ProviderAgentSessionChangedDetail>).detail ?? {},
		);
	};
	window.addEventListener(PROVIDER_AGENT_SESSION_CHANGED, handleEvent);
	const unlistenBackend = listen<ProviderAgentSessionChangedDetail>(
		PROVIDER_AGENT_SESSION_CHANGED_BACKEND,
		(event) => {
			listener(event.payload ?? {});
		},
	);
	return () => {
		window.removeEventListener(PROVIDER_AGENT_SESSION_CHANGED, handleEvent);
		void unlistenBackend.then((unlisten) => unlisten());
	};
}
