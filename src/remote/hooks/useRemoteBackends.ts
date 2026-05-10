import { useCallback, useEffect, useState } from "react";
import type { BackendInfoMsg, WsMessage } from "@/types/protocol";

interface UseRemoteBackendsParams {
	subscribe: (cb: (msg: WsMessage) => void) => () => void;
	send: (msg: WsMessage) => void;
	connected: boolean;
}

export interface RemoteBackendState {
	backends: BackendInfoMsg[];
	defaultId: string | null;
	selectedBackendId: string | null;
	setSelectedBackendId: (id: string | null) => void;
	loading: boolean;
	refresh: () => void;
}

export function useRemoteBackends({
	subscribe,
	send,
	connected,
}: UseRemoteBackendsParams): RemoteBackendState {
	const [backends, setBackends] = useState<BackendInfoMsg[]>([]);
	const [defaultId, setDefaultId] = useState<string | null>(null);
	const [selectedBackendId, setSelectedBackendId] = useState<string | null>(
		null,
	);
	const [loading, setLoading] = useState(false);

	const refresh = useCallback(() => {
		if (!connected) return;
		setLoading(true);
		send({
			type: "backend_list_request",
			payload: {},
		});
	}, [send, connected]);

	useEffect(() => {
		return subscribe((msg) => {
			if (msg.type === "backend_list_response") {
				setBackends(msg.payload.backends);
				setDefaultId(msg.payload.default_id);
				setLoading(false);
				setSelectedBackendId((prev) => {
					if (prev !== null) return prev;
					return (
						msg.payload.default_id ??
						(msg.payload.backends.length > 0
							? msg.payload.backends[0].id
							: null)
					);
				});
			}
		});
	}, [subscribe]);

	useEffect(() => {
		if (connected) {
			refresh();
		} else {
			setLoading(false);
		}
	}, [connected, refresh]);

	return {
		backends,
		defaultId,
		selectedBackendId,
		setSelectedBackendId,
		loading,
		refresh,
	};
}
