import type { TerminalStreamEndpoint } from "./terminalStreamEndpoint";
import type {
	TerminalSurfaceOwner,
	TerminalSurfaceStreamItem,
} from "./terminalSurfaceStream";

export interface TerminalStreamAttachRequest {
	attachmentId: string;
	owner: TerminalSurfaceOwner;
}

export interface TerminalStreamSocketCallbacks {
	isClosedByUs(socket: WebSocket): boolean;
	onUnexpectedClose(socket: WebSocket): void;
	onStreamItem(item: TerminalSurfaceStreamItem): void;
	onStreamError(message: string): void;
}

export function openTerminalStreamSocket(
	endpoint: TerminalStreamEndpoint,
	attach: TerminalStreamAttachRequest,
	callbacks: TerminalStreamSocketCallbacks,
): Promise<WebSocket> {
	return new Promise<WebSocket>((resolve, reject) => {
		const socket = new WebSocket(endpoint.url, [endpoint.authSubprotocol]);
		let opened = false;
		let settled = false;
		socket.onopen = () => {
			opened = true;
			socket.send(
				JSON.stringify({
					type: "attach_surface",
					id: attach.attachmentId,
					owner: attach.owner,
					attachment_id: attach.attachmentId,
				}),
			);
			settled = true;
			resolve(socket);
		};
		socket.onerror = () => {
			if (!settled) {
				settled = true;
				reject(new Error("terminal stream WebSocket connection failed"));
			}
		};
		socket.onclose = () => {
			// 接続確立前にcloseしたsocketはreject（error/close先着1回）だけで完結させる。
			// ブラウザは接続失敗時にerror→closeを連鎖発火するため、ここで
			// onUnexpectedCloseを呼ぶとfallback attachとrecovery attachが二重発行される
			if (!opened) {
				if (!settled) {
					settled = true;
					reject(new Error("terminal stream WebSocket closed during connect"));
				}
				return;
			}
			if (callbacks.isClosedByUs(socket)) return;
			callbacks.onUnexpectedClose(socket);
		};
		socket.onmessage = (event) => {
			let payload: {
				status?: string;
				item?: TerminalSurfaceStreamItem;
				error?: { message?: string };
			};
			try {
				payload = JSON.parse(String(event.data));
			} catch (error) {
				console.warn("Ignoring malformed terminal stream frame:", error);
				return;
			}
			if (payload.status === "event" && payload.item) {
				callbacks.onStreamItem(payload.item);
				return;
			}
			if (payload.status === "error") {
				callbacks.onStreamError(
					`Terminal stream error: ${payload.error?.message ?? "unknown"}`,
				);
			}
		};
	});
}
