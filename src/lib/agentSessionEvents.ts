import { listen } from "@tauri-apps/api/event";

const AGENT_SESSION_REFRESH_EVENT = "agent-session-refresh";
const AGENT_SESSION_CHANGED_BACKEND_EVENT = "agent-session-changed";

interface AgentSessionChangedDetail {
	worktreePath?: string;
}

export function notifyAgentSessionChanged(worktreePath: string) {
	window.dispatchEvent(
		new CustomEvent<AgentSessionChangedDetail>(AGENT_SESSION_REFRESH_EVENT, {
			detail: { worktreePath },
		}),
	);
}

export function subscribeAgentSessionChanged(
	listener: (detail: AgentSessionChangedDetail) => void,
) {
	const handleEvent = (event: Event) => {
		listener((event as CustomEvent<AgentSessionChangedDetail>).detail ?? {});
	};
	window.addEventListener(AGENT_SESSION_REFRESH_EVENT, handleEvent);
	const unlistenBackend = listen<AgentSessionChangedDetail>(
		AGENT_SESSION_CHANGED_BACKEND_EVENT,
		(event) => {
			listener(event.payload ?? {});
		},
	);
	return () => {
		window.removeEventListener(AGENT_SESSION_REFRESH_EVENT, handleEvent);
		void unlistenBackend.then((unlisten) => unlisten());
	};
}
