import type { Page } from "@playwright/test";

export interface MockConfig {
	/**
	 * cmd → 返り値のマッピング。関数はシリアライズできないため使用不可。
	 * `{ __mockSequence: [...] }` は呼び出し順に返し、末尾へ到達後は最後の値を維持する。
	 */
	ipcHandler: Record<string, unknown>;
}

interface TauriMockInternals {
	invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
	transformCallback: (cb: (data: unknown) => void, once?: boolean) => number;
	unregisterCallback: (id: number) => void;
	runCallback: (id: number, data: unknown) => void;
	callbacks: Map<number, { cb: (data: unknown) => void; once: boolean }>;
	invocations: Array<{ cmd: string; args: Record<string, unknown> }>;
	setMockResponse: (cmd: string, value: unknown) => void;
	metadata: {
		currentWindow: { label: string };
		currentWebview: { windowLabel: string; label: string };
	};
	convertFileSrc: (path: string) => string;
}

interface TauriEventPluginInternals {
	unregisterListener: (event: string, id: number) => void;
}

declare global {
	interface Window {
		__TAURI_INTERNALS__?: TauriMockInternals;
		__TAURI_EVENT_PLUGIN_INTERNALS__?: TauriEventPluginInternals;
	}
}

/**
 * page.addInitScript() で window.__TAURI_INTERNALS__ を注入し、
 * 全 Tauri IPC 呼び出しをモックする。
 *
 * ページナビゲーション前に呼ぶこと。
 */
export async function setupTauriMock(page: Page, config: MockConfig) {
	await page.addInitScript((cfg: MockConfig) => {
		const callbacks = new Map<
			number,
			{ cb: (data: unknown) => void; once: boolean }
		>();
		let nextId = 1;

		function transformCallback(
			cb: (data: unknown) => void,
			once = false,
		): number {
			const id = nextId++;
			callbacks.set(id, { cb, once });
			return id;
		}

		function unregisterCallback(id: number): void {
			callbacks.delete(id);
		}

		function runCallback(id: number, data: unknown): void {
			const entry = callbacks.get(id);
			if (entry) {
				if (entry.once) callbacks.delete(id);
				entry.cb(data);
			}
		}

		// イベントリスナー管理
		const eventListeners = new Map<string, number[]>();
		const invocationCounts = new Map<string, number>();
		const agentSessionNotices = new Map<
			string,
			{ operation: string; message: string }
		>();
		let agentSessionNoticeRevision = 0;
		const invocations: Array<{
			cmd: string;
			args: Record<string, unknown>;
		}> = [];

		async function invoke(
			cmd: string,
			args: Record<string, unknown> = {},
		): Promise<unknown> {
			invocations.push({ cmd, args });
			// plugin:event 系のハンドリング
			if (cmd === "plugin:event|listen") {
				const event = args.event as string;
				const handler = args.handler as number;
				if (!eventListeners.has(event)) eventListeners.set(event, []);
				eventListeners.get(event)!.push(handler);
				return handler;
			}
			if (cmd === "plugin:event|unlisten") return;
			if (cmd === "plugin:event|emit") {
				const event = args.event as string;
				const payload = args.payload;
				for (const handlerId of eventListeners.get(event) || []) {
					runCallback(handlerId, { event, payload });
				}
				return;
			}

			// list_branches_with_status_snapshot は明示ハンドラが無い場合、
			// list_branches_with_status の配列を BranchCardsSnapshot 形に
			// ラップして返す（既存フィクスチャの override をそのまま活かす）。
			if (
				cmd === "list_branches_with_status_snapshot" &&
				!(cmd in cfg.ipcHandler) &&
				"list_branches_with_status" in cfg.ipcHandler
			) {
				const branches = cfg.ipcHandler.list_branches_with_status;
				return {
					version: 1,
					stale: false,
					loading: false,
					limited: false,
					branches: Array.isArray(branches) ? branches : [],
				};
			}

			// ユーザー定義コマンド
			if (cmd in cfg.ipcHandler) {
				let value = cfg.ipcHandler[cmd];
				if (
					value &&
					typeof value === "object" &&
					"__mockSequence" in (value as Record<string, unknown>)
				) {
					const sequence = (value as { __mockSequence: unknown[] })
						.__mockSequence;
					const index = invocationCounts.get(cmd) ?? 0;
					invocationCounts.set(cmd, index + 1);
					value =
						sequence.length === 0
							? null
							: sequence[Math.min(index, sequence.length - 1)];
				}
				// { __mockError: "message" } の場合はエラーを投げる
				if (
					value &&
					typeof value === "object" &&
					"__mockError" in (value as Record<string, unknown>)
				) {
					throw new Error(
						(value as { __mockError: string }).__mockError,
					);
				}
				return value;
			}

			if (
				cmd === "get_agent_session_notice" ||
				cmd === "update_agent_session_notice"
			) {
				const sessionId = args.sessionId as string;
				if (cmd === "update_agent_session_notice") {
					const update = args.update as {
						action: "failure" | "success" | "dismiss" | "remove_session";
						operation?: string;
						message?: string;
					};
					let changed = false;
					if (update.action === "failure" && update.operation && update.message) {
						agentSessionNotices.set(sessionId, {
							operation: update.operation,
							message: update.message,
						});
						changed = true;
					} else if (update.action === "success" && update.operation) {
						if (
							agentSessionNotices.get(sessionId)?.operation === update.operation
						) {
							agentSessionNotices.delete(sessionId);
							changed = true;
						}
					} else {
						changed = agentSessionNotices.delete(sessionId);
					}
					if (changed) agentSessionNoticeRevision += 1;
				}
				const notice = agentSessionNotices.get(sessionId);
				return {
					sessionId,
					revision: agentSessionNoticeRevision,
					notice: notice ? { message: notice.message } : null,
				};
			}

			// 未定義コマンドはnull返却（ログ出力）
			console.warn("[tauri-mock] unhandled:", cmd, args);
			return null;
		}

		window.__TAURI_INTERNALS__ = {
			invoke,
			transformCallback,
			unregisterCallback,
			runCallback,
			callbacks,
			invocations,
			setMockResponse: (cmd: string, value: unknown) => {
				cfg.ipcHandler[cmd] = value;
				invocationCounts.delete(cmd);
			},
			metadata: {
				currentWindow: { label: "main" },
				currentWebview: { windowLabel: "main", label: "main" },
			},
			convertFileSrc: (path: string) => path,
		};

		window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
			unregisterListener: (_event: string, id: number) =>
				unregisterCallback(id),
		};
	}, config);
}

/**
 * ブラウザ側で Tauri イベントを発火させるヘルパー。
 * setupTauriMock 適用済みのページでのみ使用可能。
 */
export async function emitTauriEvent(
	page: Page,
	event: string,
	payload: unknown,
) {
	await page.evaluate(
		async ({ event, payload }) => {
			const internals = window.__TAURI_INTERNALS__;
			if (!internals) throw new Error("Tauri mock not initialized");
			await internals.invoke("plugin:event|emit", { event, payload });
		},
		{ event, payload },
	);
}
