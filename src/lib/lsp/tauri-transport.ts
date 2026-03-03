import { Channel, invoke } from "@tauri-apps/api/core";

// ConnectionState matches Monaco's IMessageTransport.state type
type ConnectionState =
	| { state: "connecting" }
	| { state: "open" }
	| { state: "closed"; error: Error | undefined };

type Message =
	| {
			jsonrpc: "2.0";
			method: string;
			params?: unknown;
			id?: number | string;
			result?: never;
	  }
	| {
			jsonrpc: "2.0";
			result?: unknown;
			error?: { code: unknown; message: string; data?: unknown };
			id: (number | string) | null;
			method?: never;
	  };

type MessageListener = (message: Message) => void;

interface IValueWithChangeEvent<T> {
	get value(): T;
	get onChange(): (listener: (e: T) => void) => { dispose(): void };
}

interface IMessageTransport {
	get state(): IValueWithChangeEvent<ConnectionState>;
	send(message: Message): Promise<void>;
	setListener(listener: MessageListener | undefined): void;
	toString(): string;
}

interface LspMessage {
	session_id: number;
	message: string;
}

/**
 * IMessageTransport implementation that communicates with LSP servers
 * via Tauri IPC (invoke + Channel).
 */
export class TauriTransport implements IMessageTransport {
	private _listener: MessageListener | undefined;
	private _pendingMessages: string[] = [];
	private _stateValue: ConnectionState = { state: "connecting" };
	private _stateListeners: Set<(e: ConnectionState) => void> = new Set();
	private _disposeListeners: Set<{ dispose(): void }> = new Set();
	readonly sessionId: number;
	readonly state: IValueWithChangeEvent<ConnectionState>;

	constructor(sessionId: number) {
		this.sessionId = sessionId;
		const self = this;
		this.state = {
			get value() {
				return self._stateValue;
			},
			get onChange() {
				return (listener: (e: ConnectionState) => void) => {
					self._stateListeners.add(listener);
					const disposable = {
						dispose: () => {
							self._stateListeners.delete(listener);
							self._disposeListeners.delete(disposable);
						},
					};
					self._disposeListeners.add(disposable);
					return disposable;
				};
			},
		};
	}

	setOpen(): void {
		this._stateValue = { state: "open" };
		for (const listener of this._stateListeners) {
			listener(this._stateValue);
		}
	}

	setClosed(error?: Error): void {
		this._stateValue = { state: "closed", error };
		for (const listener of this._stateListeners) {
			listener(this._stateValue);
		}
	}

	handleMessage(raw: string): void {
		if (!this._listener) {
			this._pendingMessages.push(raw);
			return;
		}
		try {
			const message = JSON.parse(raw) as Message;
			this._listener(message);
		} catch (e) {
			console.warn(`[TauriTransport] Failed to parse LSP message:`, e);
		}
	}

	async send(message: Message): Promise<void> {
		await invoke("lsp_send", {
			sessionId: this.sessionId,
			message: JSON.stringify(message),
			worktreePath: this._worktreePath,
		});
	}

	setListener(listener: MessageListener | undefined): void {
		this._listener = listener;
		if (listener && this._pendingMessages.length > 0) {
			const pending = this._pendingMessages.splice(0);
			for (const raw of pending) {
				this.handleMessage(raw);
			}
		}
	}

	toString(): string {
		return `TauriTransport(session=${this.sessionId})`;
	}

	dispose(): void {
		this.setClosed();
		for (const d of this._disposeListeners) {
			d.dispose();
		}
		this._disposeListeners.clear();
		this._stateListeners.clear();
		this._listener = undefined;
	}

	private _worktreePath = "";
	setWorktreePath(path: string): void {
		this._worktreePath = path;
	}
}

/**
 * Spawn an LSP server and create a TauriTransport connected to it.
 */
export async function createTauriTransport(
	worktreePath: string,
	language: string,
	command: string,
	args: string[],
): Promise<TauriTransport> {
	let transport: TauriTransport | null = null;
	const earlyMessages: string[] = [];

	const channel = new Channel<LspMessage>((msg) => {
		if (transport) {
			transport.handleMessage(msg.message);
		} else {
			earlyMessages.push(msg.message);
		}
	});

	const sessionId = await invoke<number>("spawn_lsp", {
		worktreePath,
		language,
		command,
		args,
		onMessage: channel,
	});

	transport = new TauriTransport(sessionId);
	transport.setWorktreePath(worktreePath);
	transport.setOpen();

	// Feed early messages to transport (buffered internally until setListener)
	for (const msg of earlyMessages) {
		transport.handleMessage(msg);
	}

	return transport;
}
