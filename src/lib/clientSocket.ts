import { invoke } from "@tauri-apps/api/core";
import type { WorkflowExecutionChangedPayload } from "@/types/workflow";

interface ClientEndpoint {
	url: string;
	authSubprotocol: string;
}

type PushListener = (event: {
	payload: WorkflowExecutionChangedPayload;
}) => void;

let connection: Promise<WebSocket> | null = null;
const listeners = new Set<{
	listener: PushListener;
	onReconnect: () => void;
}>();
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
const pending = new Map<
	string,
	{ resolve(value: unknown): void; reject(reason: unknown): void }
>();

function scheduleReconnect() {
	if (reconnectTimer || listeners.size === 0) return;
	reconnectTimer = setTimeout(() => {
		reconnectTimer = null;
		void connect().catch((error) => {
			console.warn("Client WebSocket reconnect failed:", error);
		});
	}, 1_000);
}

function connect(): Promise<WebSocket> {
	if (connection) return connection;
	const attempt = invoke<ClientEndpoint | null>("get_client_endpoint")
		.then((endpoint) => {
			if (!endpoint) throw new Error("Client endpoint is unavailable");
			return new Promise<WebSocket>((resolve, reject) => {
				const socket = new WebSocket(endpoint.url, [endpoint.authSubprotocol]);
				const timeout = setTimeout(() => fail(), 10_000);
				let closed = false;
				function fail() {
					if (closed) return;
					closed = true;
					clearTimeout(timeout);
					const error = new Error("Client WebSocket connection closed");
					if (connection === attempt) connection = null;
					reject(error);
					for (const request of pending.values()) request.reject(error);
					pending.clear();
					socket.close();
					scheduleReconnect();
				}
				socket.onopen = () => {
					if (closed) return;
					clearTimeout(timeout);
					resolve(socket);
					for (const entry of listeners) {
						entry.onReconnect();
					}
				};
				socket.onerror = fail;
				socket.onclose = fail;
				socket.onmessage = (event) => {
					if (closed) return;
					try {
						const frame = JSON.parse(String(event.data));
						if (frame.status === "push") {
							if (frame.event === "workflow-execution-changed") {
								for (const entry of listeners) {
									entry.listener({ payload: frame.payload });
								}
							}
							return;
						}
						const request = pending.get(frame.request_id);
						if (!request) return;
						pending.delete(frame.request_id);
						if ("error" in frame) request.reject(frame.error);
						else if ("result" in frame) request.resolve(frame.result);
						else request.reject(new Error("Invalid command response"));
					} catch (error) {
						console.error("Client WebSocket frame failed:", error);
						fail();
					}
				};
			});
		})
		.catch((error) => {
			if (connection === attempt) connection = null;
			scheduleReconnect();
			throw error;
		});
	connection = attempt;
	return attempt;
}

export async function invokeClient<T>(
	command: string,
	args: Record<string, unknown>,
): Promise<T> {
	const socket = await connect();
	const requestId = crypto.randomUUID();
	return new Promise<T>((resolve, reject) => {
		const timeout = setTimeout(() => {
			pending.delete(requestId);
			reject(new Error(`Client command timed out: ${command}`));
		}, 10_000);
		pending.set(requestId, {
			resolve: (value) => {
				clearTimeout(timeout);
				resolve(value as T);
			},
			reject: (error) => {
				clearTimeout(timeout);
				reject(error);
			},
		});
		try {
			socket.send(JSON.stringify({ request_id: requestId, command, args }));
		} catch (error) {
			pending.delete(requestId);
			clearTimeout(timeout);
			reject(error);
		}
	});
}

export async function listenClient(
	_event: "workflow-execution-changed",
	listener: PushListener,
	onReconnect: () => void,
): Promise<() => void> {
	const entry = { listener, onReconnect };
	listeners.add(entry);
	void connect().catch((error) => {
		console.warn("Client WebSocket subscription connection failed:", error);
	});
	return () => {
		listeners.delete(entry);
		if (listeners.size === 0 && reconnectTimer) {
			clearTimeout(reconnectTimer);
			reconnectTimer = null;
		}
	};
}
