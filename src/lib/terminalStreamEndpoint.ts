import { createCachedInvoke } from "./cachedInvoke";

export interface TerminalStreamEndpoint {
	url: string;
	authSubprotocol: string;
}

const cachedEndpoint = createCachedInvoke<
	TerminalStreamEndpoint | null,
	TerminalStreamEndpoint | null
>({
	command: "get_terminal_stream_endpoint",
	normalize: (endpoint) =>
		endpoint && typeof endpoint.url === "string" && endpoint.url.length > 0
			? endpoint
			: null,
	fallback: null,
	failureMessage:
		"Failed to resolve terminal stream endpoint, falling back to Tauri Channel:",
});

/**
 * terminal streamをWebSocketで購読するための接続情報。
 * 利用不可（local API未起動・switchで無効化・旧backend）の場合はnullを返し、
 * 呼び出し側はTauri Channel transportへfallbackする。
 */
export function getTerminalStreamEndpoint(): Promise<TerminalStreamEndpoint | null> {
	return cachedEndpoint.get();
}

export function resetTerminalStreamEndpointCache(): void {
	cachedEndpoint.reset();
}
