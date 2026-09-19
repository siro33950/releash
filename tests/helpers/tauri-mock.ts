import { clientJson } from "../../src/lib/clientJson";
import {
	create,
	fromBinary,
	fromJson,
	toBinary,
	toJson,
} from "@bufbuild/protobuf";
import {
	EnvelopeSchema,
	CommandRequestSchema,
	CommandResultSchema,
	CommandErrorSchema,
	PushSchema,
	TerminalEventSchema,
} from "../../src/generated/client_pb";
import type { TerminalSurfaceStreamItem } from "../../src/lib/terminalSurfaceStream";
import type { Page, WebSocketRoute } from "@playwright/test";

export interface MockConfig {
	/**
	 * cmd → 返り値のマッピング。関数はシリアライズできないため使用不可。
	 */
	responses: Record<string, unknown>;
}

export function workspaceTreeReconciliation(snapshot: unknown): unknown {
	return { __workspaceTreeReconciliationSnapshot: snapshot };
}

interface TauriMockInternals {
	ipcInvocations: Array<{ cmd: string; args: Record<string, unknown> }>;
	invoke: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
	transformCallback: (cb: (data: unknown) => void, once?: boolean) => number;
	unregisterCallback: (id: number) => void;
	runCallback: (id: number, data: unknown) => void;
	callbacks: Map<number, { cb: (data: unknown) => void; once: boolean }>;
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
		__releashPush: (event: string, payload: unknown) => Promise<void>;
		__releashTerminalEvent: (
			attachmentId: string,
			item: TerminalSurfaceStreamItem,
		) => Promise<void>;
		__RELEASH_BACKEND__?: {
			execute: (
				cmd: string,
				args?: Record<string, unknown>,
			) => Promise<unknown>;
			invocations: Array<{ cmd: string; args: Record<string, unknown> }>;
			setMockResponse: (cmd: string, value: unknown) => void;
		};
		__TAURI_INTERNALS__?: TauriMockInternals;
		__TAURI_EVENT_PLUGIN_INTERNALS__?: TauriEventPluginInternals;
	}
}

/**
 * WS backend fixture と UI shell の Tauri IPC mock を設定する。
 *
 * ページナビゲーション前に呼ぶこと。
 */
export async function setupTauriMock(page: Page, config: MockConfig) {
	const endpoint = {
		url: "ws://127.0.0.1:19799/v1/client",
		authSubprotocol: "releash-bearer.test-client",
	};
	config = {
		...config,
		responses: { __clientEndpoint: endpoint, __clientHello: Array.from(toBinary(EnvelopeSchema, fromJson(EnvelopeSchema, { hello: {} }))), ...config.responses },
	};
	const clientRequests: Array<{
		request_id: string;
		command: string;
		args: Record<string, unknown>;
	}> = [];
	const clients = new Set<WebSocketRoute>();
	const attachments = new Map<
		string,
		{ socket: WebSocketRoute; sequence: bigint }
	>();
	const send = (
		socket: WebSocketRoute,
		body: Parameters<typeof fromJson<typeof EnvelopeSchema>>[1],
	) => {
		socket.send(
			Buffer.from(toBinary(EnvelopeSchema, fromJson(EnvelopeSchema, body))),
		);
	};
	await page.exposeFunction(
		"__releashTerminalEvent",
		(attachmentId: string, item: TerminalSurfaceStreamItem) => {
			const attachment = attachments.get(attachmentId);
			if (!attachment) return;
			const event =
				item.type === "snapshot"
					? {
							snapshot: {
								sessionKey: item.surface.session_key,
								...item.surface.terminal_surface,
								sequence: String(item.surface.terminal_surface.sequence),
								isExited: item.surface.is_exited,
								exitCode: item.surface.exit_code,
							},
						}
					: {
							[item.type === "input_unavailable"
								? "inputUnavailable"
								: item.type]: {
								...item,
								type: undefined,
								sequence:
									"sequence" in item ? String(item.sequence) : undefined,
							},
						};
			const data = toBinary(
				TerminalEventSchema,
				fromJson(TerminalEventSchema, JSON.parse(JSON.stringify(event))),
			);
			for (let offset = 0; offset < data.length; offset += 60 * 1024) {
				attachment.sequence += 1n;
				const end = offset + 60 * 1024 >= data.length;
				const envelope = create(EnvelopeSchema, {
					body: {
						case: "stream",
						value: {
							attachmentId,
							sequence: attachment.sequence,
							data: data.slice(offset, offset + 60 * 1024),
							end,
						},
					},
				});
				attachment.socket.send(Buffer.from(toBinary(EnvelopeSchema, envelope)));
			}
		},
	);
	await page.exposeFunction(
		"__releashPush",
		(event: string, payload: unknown) => {
			const field = PushSchema.fields.find(
				(field) => field.name.replaceAll("_", "-") === event,
			);
			if (!field?.message) throw new Error(`Unknown client event: ${event}`);
			for (const socket of clients)
				send(socket, {
					push: { [field.jsonName]: clientJson(field.message, payload, true) },
				});
		},
	);
	await page.routeWebSocket(endpoint.url, (socket) => {
		clients.add(socket);
		socket.onClose(() => {
			clients.delete(socket);
			for (const [id, attachment] of attachments)
				if (attachment.socket === socket) attachments.delete(id);
		});
		socket.onMessage(async (message) => {
			if (typeof message === "string")
				throw new Error("Client requests must be binary");
			const { body } = fromBinary(EnvelopeSchema, message);
			if (body.case === "hello") {
				send(socket, { hello: { instanceId: "fixture-backend" } });
				return;
			}
			if (body.case === "heartbeat") {
				send(socket, { heartbeat: { nonce: body.value.nonce } });
				return;
			}
			if (body.case === "operationQuery") {
				send(socket, { operationStatus: { requestId: body.value.requestId, queryId: body.value.queryId, state: body.value.sent ? "unknown" : "ready" } });
				return;
			}
			if (body.case === "requestAck") {
                if (body.value.confirmWatch) send(socket, { operationStatus: { requestId: body.value.requestId, state: "watch_active" } });
                return;
            }
			if (body.case === "ack") {
				const attachment = attachments.get(body.value.attachmentId);
				if (!attachment) return;
				if (body.value.outputSequence !== undefined) {
					await page.evaluate(
						({ attachmentId, sequence }) =>
							window.__RELEASH_BACKEND__?.execute(
								"ack_terminal_surface_output",
								{ attachmentId, sequence },
							),
						{
							attachmentId: body.value.attachmentId,
							sequence: Number(body.value.outputSequence),
						},
					);
				}
				return;
			}
			if (body.case !== "request")
				throw new Error("Client request envelope required");
			const selection = body.value.command;
			if (!selection.case) throw new Error("Client command required");
			const field = CommandRequestSchema.field[selection.case];
			const command = field.name;
			const args = clientJson(
				field.message,
				toJson(field.message, selection.value),
				false,
			) as Record<string, unknown>;
			const request = { request_id: body.value.requestId, command, args };
			clientRequests.push(request);
			if (command === "attach_terminal_surface")
				attachments.set(String(args.attachmentId), { socket, sequence: 0n });
			if (command === "detach_terminal_surface")
				attachments.delete(String(args.attachmentId));
			const result = await page.evaluate(
				async ({ command, args }) => {
					try {
						return {
							result: await window.__RELEASH_BACKEND__!.execute(command, args),
						};
					} catch (error) {
						return { error: error instanceof Error ? error.message : error };
					}
				},
				{ command, args },
			);
			const outcome =
				"error" in result
					? { error: clientJson(CommandErrorSchema, result.error, true) }
					: (() => {
							const field = CommandResultSchema.fields.find(
								(field) => field.name === command,
							)!;
							try {
								return {
									result: {
										[field.jsonName]: clientJson(
											field.message!,
											result.result ?? null,
											true,
										),
									},
								};
							} catch (error) {
								throw new Error(`Invalid ${command} fixture: ${error}`);
							}
						})();
			send(socket, { response: { requestId: request.request_id, ...outcome } });
		});
	});
	await page.addInitScript((cfg: MockConfig) => {
		const callbacks = new Map<
			number,
			{ cb: (data: unknown) => void; once: boolean }
		>();
		let nextId = 1;
        let desktopSocket: WebSocket | null = null;
        let desktopAttachment = "";

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
		let terminalPerformanceStarted = false;
		let terminalPerformanceCompleted = false;
		let terminalPerformanceSequence = 0;
		let acknowledgeTerminalPerformanceOutput:
			| ((sequence: number) => void)
			| null = null;
		let startTerminalPerformanceFixture: (() => void) | null = null;
		let emitTerminalPerformanceOutput: ((data: string) => void) | null = null;
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

		async function executeCommand(
			cmd: string,
			args: Record<string, unknown> = {},
		): Promise<unknown> {
            invocations.push({ cmd, args });
            if (cmd === "admit_client_command") return null;
            if (cmd === "attach_desktop_client") {
                desktopSocket?.close();
                desktopAttachment = String(args.attachmentId);
                const endpoint = cfg.responses.__clientEndpoint as {url: string; authSubprotocol: string};
                const channel = args.channel as { onmessage: (bytes: number[] | null) => void };
                return new Promise<number[]>((resolve, reject) => {
                    const socket = new WebSocket(endpoint.url, [endpoint.authSubprotocol]);
                    desktopSocket = socket;
                    socket.binaryType = "arraybuffer";
                    let hello = true;
                    socket.onopen = () => socket.send(new Uint8Array(cfg.responses.__clientHello as number[]));
                    socket.onerror = () => reject(new Error("Fixture client connection failed"));
                    socket.onclose = () => channel.onmessage(null);
                    socket.onmessage = event => {
                        const bytes = Array.from(new Uint8Array(event.data));
                        if (hello) { hello = false; resolve(bytes); }
                        else channel.onmessage(bytes);
                    };
                });
            }
            if (cmd === "send_desktop_client_frame") {
                if (!desktopSocket || desktopSocket.readyState !== WebSocket.OPEN) throw { state: "not_sent", reason: "Fixture client disconnected" };
                desktopSocket.send(new Uint8Array(args.bytes as number[]));
                return null;
            }
            if (cmd === "detach_desktop_client") {
                if (args.attachmentId === desktopAttachment) { desktopSocket?.close(); desktopSocket = null; }
                return null;
            }
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
				!(cmd in cfg.responses)
			) {
				return {
					disableOutputFlowControl: false,
					disableTerminalJournal: false,
					disableRendererWriteSerialization: false,
					disableWebglRenderer: true,
				};
			}

			// list_branches_with_status_snapshot は明示ハンドラが無い場合、
			// list_branches_with_status の配列を BranchCardsSnapshot 形に
			// ラップして返す（既存フィクスチャの override をそのまま活かす）。
			if (
				cmd === "list_branches_with_status_snapshot" &&
				!(cmd in cfg.responses) &&
				"list_branches_with_status" in cfg.responses
			) {
				const branches = cfg.responses.list_branches_with_status;
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
						working_areas: worktreeCards,
					},
				};
			}

			// ユーザー定義コマンド
			if (cmd in cfg.responses) {
				let value = cfg.responses[cmd];
				if (
					cmd === "attach_terminal_surface" &&
					value &&
					typeof value === "object" &&
					("__mockTerminalAttachment" in (value as Record<string, unknown>) ||
						"__mockTerminalPerformanceAttachment" in
							(value as Record<string, unknown>))
				) {
					const channelId = transformCallback((data) => {
						const { message } = data as { message: TerminalSurfaceStreamItem };
						void window.__releashTerminalEvent(
							String(args.attachmentId),
							message,
						);
					});
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
									surface: {
										...(cfg.responses.get_terminal_surface as object),
										terminal_surface: {
											replay: "",
											sequence: 0,
											cols: 80,
											rows: 24,
										},
									},
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
					throw new Error((value as { __mockError: string }).__mockError);
				}
				return value;
			}

			if (cmd === "get_daemon_status") return { phase: "ready" };
			if (cmd === "list_client_handoff") return [];
			if (["validate_daemon_connection", "forget_client_operation"].includes(cmd)) return null;
			if (cmd === "get_login_item_status") return { enabled: false, requiresApproval: false, reason: null };
			if (cmd === "check_desktop_update") return null;
			if (cmd === "get_application_startup_outcome") {
				return { type: "ready" };
			}


			// 未定義コマンドはnull返却（ログ出力）
			console.warn("[tauri-mock] unhandled:", cmd, args);
			return null;
		}

		const ipcInvocations: Array<{
			cmd: string;
			args: Record<string, unknown>;
		}> = [];
		window.__RELEASH_BACKEND__ = {
			execute: executeCommand,
			invocations,
			setMockResponse: (cmd, value) => {
				cfg.responses[cmd] = value;
			},
		};
		window.__TAURI_INTERNALS__ = {
			invoke: (cmd, args = {}) => {
				ipcInvocations.push({ cmd, args });
				if (
					!cmd.startsWith("plugin:") &&
					![
						"get_daemon_status", "retry_daemon", "quit_desktop", "restart_desktop", "validate_daemon_connection", "get_login_item_status", "open_login_item_settings", "install_cli", "set_login_item_enabled", "check_desktop_update", "install_desktop_update", "forget_client_operation", "list_client_handoff", "apply_desktop_settings",
						"complete_desktop_restoration", "fail_desktop_restoration", "admit_client_command", "attach_desktop_client", "detach_desktop_client", "send_desktop_client_frame",
						"get_application_startup_outcome",
						"quit_after_startup_failure",
						"set_menu_items_enabled",
					].includes(cmd)
				)
					throw new Error(`Backend command used Tauri IPC: ${cmd}`);
				return executeCommand(cmd, args);
			},
			ipcInvocations,
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

		window.__TAURI_EVENT_PLUGIN_INTERNALS__ = {
			unregisterListener: (_event: string, id: number) =>
				unregisterCallback(id),
		};
	}, config);
	return {
		clientRequests,
		push: (event: string, payload: unknown) => {
			const field = PushSchema.fields.find(
				(field) => field.name.replaceAll("_", "-") === event,
			);
			if (!field) throw new Error(`Unknown client event: ${event}`);
			const push = {
				[field.jsonName]: clientJson(field.message!, payload, true),
			};
			for (const socket of clients) send(socket, { push });
		},
	};
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
			if (["menu-event", "native-file-drop"].includes(event))
				await internals.invoke("plugin:event|emit", { event, payload });
			else await window.__releashPush(event, payload);
		},
		{ event, payload },
	);
}
