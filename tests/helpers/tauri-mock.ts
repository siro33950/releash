import type { Page } from "@playwright/test";

export interface MockConfig {
	/**
	 * cmd → 返り値のマッピング。関数はシリアライズできないため使用不可。
	 */
	ipcHandler: Record<string, unknown>;
}

export function workspaceTreeReconciliation(snapshot: unknown): unknown {
	return { __workspaceTreeReconciliationSnapshot: snapshot };
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
		const agentSessionNotices = new Map<
			string,
			{ operation: string; message: string }
		>();
		let terminalPerformanceStarted = false;
		let terminalPerformanceCompleted = false;
		let terminalPerformanceSequence = 0;
		let acknowledgeTerminalPerformanceOutput:
			| ((sequence: number) => void)
			| null = null;
		let startTerminalPerformanceFixture: (() => void) | null = null;
		let emitTerminalPerformanceOutput: ((data: string) => void) | null = null;
		let agentSessionNoticeRevision = 0;
		const invocations: Array<{
			cmd: string;
			args: Record<string, unknown>;
		}> = [];

		function workspaceTreeContainsNode(
			nodes: unknown,
			selectedNodeId: string,
		): boolean {
			if (!Array.isArray(nodes)) return false;
			return nodes.some((item) => {
				if (!item || typeof item !== "object") return false;
				const record = item as Record<string, unknown>;
				if (record.kind === "node") return record.id === selectedNodeId;
				return workspaceTreeContainsNode(record.children, selectedNodeId);
			});
		}

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
			if (cmd === "ack_terminal_surface_output") {
				acknowledgeTerminalPerformanceOutput?.(Number(args.sequence));
				return;
			}
			if (cmd === "start_terminal_performance_fixture") {
				startTerminalPerformanceFixture?.();
				return;
			}
			if (cmd === "write_terminal_surface" && emitTerminalPerformanceOutput) {
				emitTerminalPerformanceOutput(String(args.data ?? ""));
				return;
			}
			// Chromiumで走るmockテストはDOM span/CSSのassertを維持するため
			// DOMレンダラを明示する（WebGL既定の実機経路はwdio harnessが担う）。
			if (
				cmd === "get_terminal_performance_switches" &&
				!(cmd in cfg.ipcHandler)
			) {
				return {
					disableOutputFlowControl: false,
					disableTerminalJournal: false,
					disableTerminalWebsocket: false,
					disableRendererWriteSerialization: false,
					disableWebglRenderer: true,
				};
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
				const cards = Array.isArray(branches) ? branches : [];
				const worktreeCards = cards.filter(
					(card: Record<string, unknown>) => card.worktree_path != null,
				);
				return {
					version: 1,
					stale: false,
					loading: false,
					limited: false,
					branches: cards,
					// backend が確定する表示グループ。fixture は作業の場だけを持つ。
					worktree_display_groups: {
						working_areas: worktreeCards.filter(
							(card: Record<string, unknown>) =>
								card.management_kind === "working_area",
						),
						cleanup_candidates: worktreeCards.filter(
							(card: Record<string, unknown>) =>
								card.management_kind === "cleanup_candidate" ||
								card.management_kind === "untracked_cleanup_candidate",
						),
					},
				};
			}

			// ユーザー定義コマンド
			if (cmd in cfg.ipcHandler) {
				let value = cfg.ipcHandler[cmd];
				if (
					cmd === "attach_terminal_surface" &&
					value &&
					typeof value === "object" &&
					("__mockTerminalAttachment" in (value as Record<string, unknown>) ||
						"__mockTerminalPerformanceAttachment" in
							(value as Record<string, unknown>))
				) {
					const rawChannel = args.onEvent;
					const channel =
						typeof rawChannel === "string"
							? rawChannel
							: rawChannel &&
									typeof rawChannel === "object" &&
									"id" in rawChannel
								? `__CHANNEL__:${String((rawChannel as { id: unknown }).id)}`
								: "";
					const channelId = /^__CHANNEL__:(\d+)$/.exec(channel)?.[1];
					if (!channelId) {
						throw new Error("attach_terminal_surface requires a Tauri Channel");
					}
					if (
						"__mockTerminalPerformanceAttachment" in
						(value as Record<string, unknown>)
					) {
						const config = (
							value as {
							__mockTerminalPerformanceAttachment: {
								targetBytes: number;
								chunkCodeUnits: number;
								initialReplay?: string;
								};
							}
						).__mockTerminalPerformanceAttachment;
						const sessionKey = "workspace:10:/test/repo";
						queueMicrotask(() => {
							runCallback(Number(channelId), {
								index: 0,
								message: {
									type: "snapshot",
									surface: {
										session_key: sessionKey,
										terminal_surface: {
											replay: terminalPerformanceCompleted
												? "\u001bcPERF-FIXTURE-COMPLETE"
												: (config.initialReplay ?? ""),
											sequence: terminalPerformanceSequence,
											cols: 80,
											rows: 24,
										},
										is_exited: false,
										exit_code: null,
									},
								},
							});
						});
						if (!startTerminalPerformanceFixture) {
							const frame =
								"\u001b[38;5;220m◆ tool\u001b[0m 日本語🙂 wide\r\n" +
								"\u001b[2K\r\u001b[32m✓ completed\u001b[0m\r\n" +
								"\u001b[2A\u001b[12C\u001b[1mredraw\u001b[0m\u001b[2B\r\n" +
								"history-line 日本語🙂\r\n";
							const frameBytes = new TextEncoder().encode(frame).byteLength;
							const fixture = frame.repeat(
								Math.ceil(config.targetBytes / frameBytes),
							);
							const performanceState = (
								window as typeof window & {
									__RELEASH_TERMINAL_PERFORMANCE_STATE__?: {
										fixtureByteLength: number;
									};
								}
							).__RELEASH_TERMINAL_PERFORMANCE_STATE__;
							if (performanceState) {
								performanceState.fixtureByteLength = new TextEncoder().encode(
									fixture,
								).byteLength;
							}
							let offset = 0;
							let pendingCodeUnits = 0;
							let pending: Array<{ sequence: number; codeUnits: number }> = [];
							let continuationPosted = false;
							const continuation = new MessageChannel();
							const schedule = () => {
								if (
									!terminalPerformanceStarted ||
									continuationPosted ||
									offset >= fixture.length ||
									pendingCodeUnits >= 256 * 1024
								)
									return;
								continuationPosted = true;
								continuation.port2.postMessage(null);
							};
							acknowledgeTerminalPerformanceOutput = (sequence) => {
								pending = pending.filter((entry) => {
									if (entry.sequence > sequence) return true;
									pendingCodeUnits -= entry.codeUnits;
									return false;
								});
								schedule();
							};
							emitTerminalPerformanceOutput = (data) => {
								if (!data) return;
								terminalPerformanceSequence += 1;
								runCallback(Number(channelId), {
									index: terminalPerformanceSequence,
									message: {
										type: "output",
										session_key: sessionKey,
										data,
										sequence: terminalPerformanceSequence,
									},
								});
							};
							startTerminalPerformanceFixture = () => {
								if (terminalPerformanceStarted) return;
								terminalPerformanceStarted = true;
								const state = (
									window as typeof window & {
										__RELEASH_TERMINAL_PERFORMANCE_STATE__?: {
											fixtureStartedAt: number;
										};
									}
								).__RELEASH_TERMINAL_PERFORMANCE_STATE__;
								if (state) state.fixtureStartedAt = performance.now();
								schedule();
							};
							continuation.port1.onmessage = () => {
								continuationPosted = false;
								for (
									let index = 0;
									index < 8 &&
									offset < fixture.length &&
									pendingCodeUnits < 256 * 1024;
									index += 1
								) {
									const data = fixture.slice(
										offset,
										offset + config.chunkCodeUnits,
									);
									offset += data.length;
									terminalPerformanceSequence += 1;
									pending.push({
										sequence: terminalPerformanceSequence,
										codeUnits: data.length,
									});
									pendingCodeUnits += data.length;
									runCallback(Number(channelId), {
										index: terminalPerformanceSequence,
										message: {
											type: "output",
											session_key: sessionKey,
											data,
											sequence: terminalPerformanceSequence,
										},
									});
								}
								if (offset < fixture.length) {
									schedule();
									return;
								}
								terminalPerformanceSequence += 1;
								terminalPerformanceCompleted = true;
								runCallback(Number(channelId), {
									index: terminalPerformanceSequence,
									message: {
										type: "output",
										session_key: sessionKey,
										data: "\r\nPERF-FIXTURE-COMPLETE",
										sequence: terminalPerformanceSequence,
									},
								});
								continuation.port1.close();
								continuation.port2.close();
							};
						}
						return null;
					}
					const attachment = value as {
						messages?: unknown[];
					};
					const messages = Array.isArray(attachment.messages)
						? attachment.messages
						: [
								{
									type: "snapshot",
									surface: cfg.ipcHandler.get_terminal_surface,
								},
							];
					queueMicrotask(() => {
						for (const [index, message] of messages.entries()) {
							runCallback(Number(channelId), { index, message });
						}
					});
					return null;
				}
				if (
					value &&
					typeof value === "object" &&
					"__mockAcceptedPermissionResponse" in
						(value as Record<string, unknown>)
				) {
					const operationId = args.operationId as string;
					const requestId = args.requestId as string;
					return {
						type: "accepted",
						operation: {
							receipt: {
								operation_id: operationId,
								session_id: args.chatSessionId as string,
								request_id: requestId,
								input_ref: `permission-response:${requestId}`,
							},
							latest_status: {
								type: "completed",
								decision: args.behavior === "allow" ? "allowed" : "denied",
							},
						},
					};
				}
				if (
					cmd === "get_workspace_tree_selection_reconciliation" &&
					value &&
					typeof value === "object" &&
					"__workspaceTreeReconciliationSnapshot" in
						(value as Record<string, unknown>)
				) {
					const snapshot = (
						value as { __workspaceTreeReconciliationSnapshot: unknown }
					).__workspaceTreeReconciliationSnapshot as Record<string, unknown>;
					const selectedNodeId = args.selectedNodeId as string;
					return {
						snapshot,
						reconciliation: {
							selectionInSnapshot: workspaceTreeContainsNode(
								snapshot.nodes,
								selectedNodeId,
							),
						},
					};
				}
				if (
					value &&
					typeof value === "object" &&
					"__mockAcceptedStop" in (value as Record<string, unknown>)
				) {
					const request = args.request as {
						request_id: string;
						session_id: string;
						turn_id: string;
						expected_session_revision: string;
					};
					return {
						type: "accepted",
						receipt: {
							operation_id: request.request_id,
							session_id: request.session_id,
							turn_id: request.turn_id,
							accepted_revision: request.expected_session_revision,
						},
						state: { type: "accepted" },
					};
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

			if (cmd === "get_application_startup_outcome") {
				return { type: "ready" };
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
