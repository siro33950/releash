import type { Page } from "@playwright/test";

export interface MockConfig {
	/** cmd → 返り値のマッピング。関数の場合はシリアライズできないため、静的な値のみ対応 */
	ipcHandler: Record<string, unknown>;
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

		async function invoke(
			cmd: string,
			args: Record<string, unknown> = {},
		): Promise<unknown> {
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
				const value = cfg.ipcHandler[cmd];
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

			// 未定義コマンドはnull返却（ログ出力）
			console.warn("[tauri-mock] unhandled:", cmd, args);
			return null;
		}

		// @ts-expect-error - グローバルにTauriモック構造を注入
		window.__TAURI_INTERNALS__ = {
			invoke,
			transformCallback,
			unregisterCallback,
			runCallback,
			callbacks,
			metadata: {
				currentWindow: { label: "main" },
				currentWebview: { windowLabel: "main", label: "main" },
			},
			convertFileSrc: (path: string) => path,
		};

		// @ts-expect-error - イベントプラグインの内部構造
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
		({ event, payload }) => {
			// @ts-expect-error - __TAURI_INTERNALS__ は setupTauriMock で注入済み
			const internals = window.__TAURI_INTERNALS__;
			if (!internals) throw new Error("Tauri mock not initialized");
			internals.invoke("plugin:event|emit", { event, payload });
		},
		{ event, payload },
	);
}
