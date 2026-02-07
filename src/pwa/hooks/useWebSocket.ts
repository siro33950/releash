import { hmac } from "@noble/hashes/hmac.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { bytesToHex } from "@noble/hashes/utils.js";
import { useCallback, useEffect, useRef, useState } from "react";
import type { WsMessage } from "@/types/protocol";
import { deserializeMessage, serializeMessage } from "@/types/protocol";

export type ConnectionStatus = "disconnected" | "connecting" | "authenticating" | "connected";

interface UseWebSocketOptions {
	url: string;
	token: string;
	onMessage?: (msg: WsMessage) => void;
	onStatusChange?: (status: ConnectionStatus) => void;
}

const MAX_BACKOFF_MS = 30_000;
const INITIAL_BACKOFF_MS = 1_000;

function computeHmac(challenge: string, token: string): string {
	const enc = new TextEncoder();
	return bytesToHex(hmac(sha256, enc.encode(token), enc.encode(challenge)));
}

export function useWebSocket({ url, token, onMessage, onStatusChange }: UseWebSocketOptions) {
	const [status, setStatus] = useState<ConnectionStatus>("disconnected");
	const wsRef = useRef<WebSocket | null>(null);
	const backoffRef = useRef(INITIAL_BACKOFF_MS);
	const reconnectTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const mountedRef = useRef(true);
	const onMessageRef = useRef(onMessage);
	const onStatusChangeRef = useRef(onStatusChange);
	onMessageRef.current = onMessage;
	onStatusChangeRef.current = onStatusChange;

	const updateStatus = useCallback((s: ConnectionStatus) => {
		setStatus(s);
		onStatusChangeRef.current?.(s);
	}, []);

	const send = useCallback((msg: WsMessage) => {
		if (wsRef.current?.readyState === WebSocket.OPEN) {
			wsRef.current.send(serializeMessage(msg));
		}
	}, []);

	const connect = useCallback(() => {
		if (!mountedRef.current || !url || !token) return;
		if (wsRef.current) {
			wsRef.current.close();
			wsRef.current = null;
		}

		updateStatus("connecting");
		const ws = new WebSocket(url);
		wsRef.current = ws;

		ws.onopen = () => {
			updateStatus("authenticating");
		};

		ws.onmessage = async (event) => {
			let msg: WsMessage;
			try {
				msg = deserializeMessage(event.data as string);
			} catch (e) {
				console.warn("Failed to parse WebSocket message:", e);
				return;
			}

			if (msg.type === "auth_challenge") {
				const hmac = await computeHmac(msg.payload.challenge, token);
				ws.send(serializeMessage({ type: "auth_response", payload: { hmac } }));
				return;
			}

			if (msg.type === "auth_result") {
				if (msg.payload.success) {
					updateStatus("connected");
					backoffRef.current = INITIAL_BACKOFF_MS;
				} else {
					ws.close();
				}
				return;
			}

			onMessageRef.current?.(msg);
		};

		ws.onclose = () => {
			wsRef.current = null;
			if (!mountedRef.current) return;
			updateStatus("disconnected");
			scheduleReconnect();
		};

		ws.onerror = () => {
			// onclose will fire after onerror
		};
	}, [url, token, updateStatus]);

	const scheduleReconnect = useCallback(() => {
		if (!mountedRef.current) return;
		const delay = backoffRef.current;
		backoffRef.current = Math.min(delay * 2, MAX_BACKOFF_MS);
		reconnectTimerRef.current = setTimeout(() => {
			connect();
		}, delay);
	}, [connect]);

	const disconnect = useCallback(() => {
		if (reconnectTimerRef.current) {
			clearTimeout(reconnectTimerRef.current);
			reconnectTimerRef.current = null;
		}
		if (wsRef.current) {
			wsRef.current.close();
			wsRef.current = null;
		}
		updateStatus("disconnected");
	}, [updateStatus]);

	useEffect(() => {
		mountedRef.current = true;
		if (url && token) {
			connect();
		}
		return () => {
			mountedRef.current = false;
			disconnect();
		};
	}, [url, token, connect, disconnect]);

	return { status, send, disconnect, reconnect: connect };
}
