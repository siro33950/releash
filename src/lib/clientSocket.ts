import { invoke } from "@tauri-apps/api/core";
import type {
	ClientCommandArgs,
	ClientCommandResults,
	ClientPushPayloads,
} from "@/generated/client_types";
import {
	ClientStreamDecoder,
	decodeClientEnvelope,
	decodeClientPush,
	decodeClientValue,
	encodeClientAck,
	encodeClientCommand,
	encodeClientRequestAck,
	MAX_STREAM_FRAME_BYTES,
} from "./clientProtocol";
import type { TerminalSurfaceStreamItem } from "./terminalSurfaceStream";

interface ClientEndpoint {
	url: string;
	authSubprotocol: string;
}

type PushListener = (event: {
	payload: ClientPushPayloads["workflow-execution-changed"];
}) => void;

let connection: Promise<WebSocket> | null = null;
let connectedSocket: WebSocket | null = null;
const connectionListeners = new Set<(connected: boolean) => void>();
const streams = new Map<
	string,
	{
		decoder: ClientStreamDecoder;
		listener: (item: TerminalSurfaceStreamItem) => void;
		onClosed: () => void;
	}
>();
const listeners = new Set<{
	listener: PushListener;
	onReconnect: () => void;
}>();
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
const pending = new Map<
	string,
	{
		command: ClientCommand;
		resolve(value: unknown): void;
		reject(reason: unknown): void;
	}
>();

function scheduleReconnect() {
	if (
		reconnectTimer ||
		(listeners.size === 0 && connectionListeners.size === 0)
	)
		return;
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
				socket.binaryType = "arraybuffer";
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
					connectedSocket = null;
					streams.clear();
					for (const listener of connectionListeners) listener(false);
					socket.close();
					scheduleReconnect();
				}
				socket.onopen = () => {
					if (closed) return;
					clearTimeout(timeout);
					connectedSocket = socket;
					resolve(socket);
					for (const listener of connectionListeners) listener(true);
					for (const entry of listeners) {
						entry.onReconnect();
					}
				};
				socket.onerror = fail;
				socket.onclose = fail;
				socket.onmessage = (event) => {
					if (closed) return;
					try {
						const { body } = decodeClientEnvelope(event.data);
						if (body.case === "pushResync") {
							for (const entry of listeners) entry.onReconnect();
							return;
						}
						if (body.case === "streamClosed") {
							const entry = streams.get(body.value.attachmentId);
							streams.delete(body.value.attachmentId);
							entry?.onClosed();
							return;
						}
						if (body.case === "push") {
							if (body.value.event.case === "workflowExecutionChanged") {
								const payload = decodeClientPush(
									body.value,
									"workflow-execution-changed",
								);
								for (const entry of listeners) entry.listener({ payload });
							}
							return;
						}
						if (body.case === "stream") {
							if (event.data.byteLength > MAX_STREAM_FRAME_BYTES)
								throw new Error("Terminal frame exceeds limit");
							const entry = streams.get(body.value.attachmentId);
							if (!entry) return;
							const item = entry.decoder.decode(body.value);
							if (item) entry.listener(item);
							socket.send(
								encodeClientAck(
									body.value.attachmentId,
									Number(body.value.sequence),
								),
							);
							return;
						}
						if (body.case !== "response")
							throw new Error("Invalid client response");
						const request = pending.get(body.value.requestId);
						if (!request) {
							if (!body.value.requestId)
								throw new Error("Uncorrelated client error");
							return;
						}
						const outcome = body.value.outcome;
						if (outcome.case === "error")
							request.reject(decodeClientValue(outcome.value));
						else if (outcome.case === "result") {
							const value = decodeClientValue(outcome.value, request.command);
							if (
								request.command === "start_watching" ||
								request.command === "start_git_dir_watching"
							)
								socket.send(encodeClientRequestAck(body.value.requestId));
							request.resolve(value);
						} else request.reject(new Error("Invalid command response"));
						pending.delete(body.value.requestId);
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

export type ClientCommand = keyof ClientCommandArgs;
export type EmptyClientCommand = {
	[K in ClientCommand]: Record<string, never> extends ClientCommandArgs[K]
		? K
		: never;
}[ClientCommand];
export async function invokeClient<K extends ClientCommand>(
	command: K,
	args?: ClientCommandArgs[K],
): Promise<ClientCommandResults[K]> {
	const socket = await connect();
	const requestId = crypto.randomUUID();
	return new Promise((resolve, reject) => {
		const timeout =
			command === "get_current_branch"
				? setTimeout(() => {
						pending.delete(requestId);
						reject(new Error(`Client command timed out: ${command}`));
					}, 10_000)
				: undefined;
		pending.set(requestId, {
			command,
			resolve: (value) => {
				clearTimeout(timeout);
				resolve(value as ClientCommandResults[K]);
			},
			reject: (error) => {
				clearTimeout(timeout);
				reject(error);
			},
		});
		try {
			socket.send(encodeClientCommand(requestId, command, args ?? {}));
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
		if (
			listeners.size === 0 &&
			connectionListeners.size === 0 &&
			reconnectTimer
		) {
			clearTimeout(reconnectTimer);
			reconnectTimer = null;
		}
	};
}

export function onClientConnection(
	listener: (connected: boolean) => void,
): () => void {
	connectionListeners.add(listener);
	void connect().catch((error) =>
		console.warn("Client connection failed:", error),
	);
	return () => {
		connectionListeners.delete(listener);
		if (
			listeners.size === 0 &&
			connectionListeners.size === 0 &&
			reconnectTimer
		) {
			clearTimeout(reconnectTimer);
			reconnectTimer = null;
		}
	};
}

export function listenClientStream(
	attachmentId: string,
	listener: (item: TerminalSurfaceStreamItem) => void,
	onClosed: () => void,
): () => void {
	streams.set(attachmentId, {
		decoder: new ClientStreamDecoder(),
		listener,
		onClosed,
	});
	return () => {
		streams.delete(attachmentId);
	};
}

export function acknowledgeClientStream(
	attachmentId: string,
	sequence: number,
) {
	if (connectedSocket)
		connectedSocket.send(encodeClientAck(attachmentId, 0, sequence));
}
