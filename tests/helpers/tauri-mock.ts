import { createServer } from "node:http";
import { once } from "node:events";
import { clientJson } from "../../src/lib/clientJson";
import { create, fromJson, toJson, type Message } from "@bufbuild/protobuf";
import { Code, ConnectError, createConnectRouter } from "@connectrpc/connect";
import { createFetchHandler } from "@connectrpc/connect/protocol";
import { ClientService, CommandRequestSchema, CommandErrorSchema, PushSchema, StateSubscriptionEventSchema, TerminalEventSchema, TerminalSubscriptionEventSchema, AttachTerminalSurfaceRequestSchema } from "../../src/generated/client_pb";
import type { WorkspaceListSnapshotDto } from "../../src/generated/client_types";
import type { TerminalSurfaceStreamItem } from "../../src/lib/terminalSurfaceStream";
import type { Page } from "@playwright/test";
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
		__releashRepositoryPaths: () => string[];
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
 * Connect backend fixture と UI shell の Tauri IPC mock を設定する。
 *
 * ページナビゲーション前に呼ぶこと。
 */
export async function setupTauriMock(page: Page, config: MockConfig) {
    const clientRequests: Array<{ request_id: string; command: string; args: Record<string, unknown> }> = [];
    const pushes = new Set<ReadableStreamDefaultController<Message>>();
    const attachments = new Map<string, { output: ReadableStreamDefaultController<Message>; streamId: string }>();
    const terminalSubscriptions = new Map<string, ReadableStreamDefaultController<Message>>();
    const stateStreams = new Map<string, ReadableStreamDefaultController<Message>>();
    const push = (event: string, payload: unknown) => {
        const field = PushSchema.fields.find(field => field.name.replaceAll("_", "-") === event);
        if (!field?.message) throw new Error(`Unknown client event: ${event}`);
        const message = fromJson(PushSchema, { [field.jsonName]: clientJson(field.message, payload, true) });
        for (const stream of pushes) stream.enqueue(message);
    };
    await page.exposeFunction("__releashPush", push);
    await page.exposeFunction("__releashTerminalEvent", (attachmentId: string, item: TerminalSurfaceStreamItem) => {
        const attachment = attachments.get(attachmentId);
        if (!attachment) return;
        const { output: stream, streamId } = attachment;
        const event = item.type === "snapshot"
            ? { snapshot: { sessionKey: item.surface.session_key, ...item.surface.terminal_surface, sequence: String(item.surface.terminal_surface.sequence), isExited: item.surface.is_exited, exitCode: item.surface.exit_code } }
            : { [item.type === "input_unavailable" ? "inputUnavailable" : item.type]: { ...item, sessionKey: item.session_key, type: undefined, session_key: undefined, exitCode: "exit_code" in item ? item.exit_code : undefined, exit_code: undefined, sequence: "sequence" in item ? String(item.sequence) : undefined } };
        stream.enqueue(create(TerminalSubscriptionEventSchema, { attachmentId, streamId, event: { case: "item", value: fromJson(TerminalEventSchema, JSON.parse(JSON.stringify(event))) } }));
        if (item.type === "exit" || (item.type === "snapshot" && item.surface.is_exited)) {
            attachments.delete(attachmentId);
            stream.enqueue(create(TerminalSubscriptionEventSchema, { attachmentId, streamId, event: { case: "closed", value: { resynchronize: false } } }));
        }
    });
    const execute = async (command: string, args: Record<string, unknown>) => {
        clientRequests.push({ request_id: crypto.randomUUID(), command, args });
        const outcome = await page.evaluate(async ({command, args}) => {
            try { return { result: await window.__RELEASH_BACKEND__!.execute(command, args) }; }
            catch (error) { return { error: error instanceof Error ? error.message : error }; }
        }, {command, args});
        if ("error" in outcome) throw new ConnectError("Command failed", Code.FailedPrecondition, undefined, [{ desc: CommandErrorSchema, value: fromJson(CommandErrorSchema, clientJson(CommandErrorSchema, outcome.error, true)) }]);
        return outcome.result;
    };
    const router = createConnectRouter();
    for (const method of ClientService.methods) {
        if (method.name === "OpenStateStream") {
            router.rpc(method, async function* (request, context) {
                let controller: ReadableStreamDefaultController<Message>;
                const stream = new ReadableStream<Message>({ start(value) { controller = value; stateStreams.set(request.clientId, value); } });
                const stop = () => controller.close();
                context.signal.addEventListener("abort", stop, { once: true });
                try { yield create(StateSubscriptionEventSchema, { event: { case: "ready", value: {} } }); yield* stream; }
                finally { stateStreams.delete(request.clientId); context.signal.removeEventListener("abort", stop); }
            });
            continue;
        }
        if (method.name === "StartStateSubscription") {
            router.rpc(method, async request => {
                const stream = stateStreams.get(request.clientId);
                if (!stream || request.target !== "repository-paths") throw new ConnectError("Unknown state subscription", Code.NotFound);
                const items = await page.evaluate(() => window.__releashRepositoryPaths());
                const version = { epoch: "fixture", sequence: 0n };
                stream.enqueue(create(StateSubscriptionEventSchema, { target: request.target, version, event: { case: "snapshot", value: { value: { case: "repositoryPaths", value: { items } } } } }));
                stream.enqueue(create(StateSubscriptionEventSchema, { target: request.target, version, event: { case: "bookmark", value: {} } }));
                return {};
            });
            continue;
        }
        if (method.name === "StopStateSubscription") { router.rpc(method, () => ({})); continue; }
        if (method.name === "GetServerInfo") { router.rpc(method, () => ({ launchId: "fixture" })); continue; }
        if (method.name === "SubscribePush") {
            router.rpc(method, async function* (_, context) {
                let controller: ReadableStreamDefaultController<Message>;
                const stream = new ReadableStream<Message>({ start(value) { controller = value; pushes.add(value); } });
                const stop = () => { pushes.delete(controller); controller.close(); };
                context.signal.addEventListener("abort", stop, { once: true });
                try { yield create(PushSchema, {event: {case: "resync", value: {}}}); yield* stream; }
                finally { pushes.delete(controller!); context.signal.removeEventListener("abort", stop); }
            });
            continue;
        }
        if (method.name === "WatchFiles" || method.name === "WatchGitDirectory") {
            const command = method.name === "WatchFiles" ? "start_watching" : "start_git_dir_watching";
            const schema = method.input.fields.find(field => field.name === "request")?.message;
            if (!schema) throw new Error(`Missing watch request schema: ${method.name}`);
            router.rpc(method, async request => {
                const args = clientJson(schema, toJson(schema, request.request!), false) as Record<string, unknown>;
                return fromJson(method.output, clientJson(method.output, await execute(command, args), true));
            });
            continue;
        }
        if (method.name === "SubscribeTerminalSurfaces") {
            router.rpc(method, async function* (request, context) {
                let controller: ReadableStreamDefaultController<Message>;
                const stream = new ReadableStream<Message>({ start(value) { controller = value; terminalSubscriptions.set(request.subscriptionId, value); } });
                const stop = () => controller.close();
                context.signal.addEventListener("abort", stop, { once: true });
                try { yield create(TerminalSubscriptionEventSchema, { event: { case: "ready", value: {} } }); yield* stream; }
                finally {
                    terminalSubscriptions.delete(request.subscriptionId);
                    for (const [id, output] of attachments) if (output.output === controller!) attachments.delete(id);
                    context.signal.removeEventListener("abort", stop);
                }
            });
            continue;
        }
        if (method.name === "AttachTerminalSurface") {
            router.rpc(method, async request => {
                const output = terminalSubscriptions.get(request.subscriptionId);
                if (!output || !request.request) throw new ConnectError("Terminal subscription ended", Code.NotFound);
                const args = clientJson(AttachTerminalSurfaceRequestSchema, toJson(AttachTerminalSurfaceRequestSchema, request.request), false) as Record<string, unknown>;
                const id = String(args.attachmentId);
                attachments.set(id, { output, streamId: request.streamId });
                try { await execute("attach_terminal_surface", args); }
                catch (error) { attachments.delete(id); throw error; }
                return {};
            });
            continue;
        }
        const command = CommandRequestSchema.fields.find(field => field.message?.typeName === method.input.typeName)!.name;
        const argsFor = (request: Message) => clientJson(method.input, toJson(method.input, request), false) as Record<string, unknown>;
        if (method.name === "DetachTerminalSurface") {
            router.rpc(method, async request => {
                const id = request.attachmentId ?? "";
                const output = attachments.get(id);
                attachments.delete(id);
                output?.output.enqueue(create(TerminalSubscriptionEventSchema, { attachmentId: id, streamId: output.streamId, event: { case: "closed", value: { resynchronize: true } } }));
                await execute(command, argsFor(request));
                return {};
            });
        } else if (method.methodKind === "server_streaming") {
            router.rpc(method, async function* (request, context) {
                const result = await execute(command, argsFor(request));
                yield fromJson(method.output, clientJson(method.output, result ?? null, true));
                if (!context.signal.aborted) await new Promise<void>(resolve => context.signal.addEventListener("abort", () => resolve(), {once:true}));
            });
        } else {
            router.rpc(method, async request => fromJson(method.output, clientJson(method.output, (await execute(command, argsFor(request))) ?? null, true)));
        }
    }
    const server = createServer(async (req, res) => {
        const origin = req.headers.origin;
        if (origin) res.setHeader("access-control-allow-origin", origin);
        res.setHeader("access-control-allow-headers", "authorization,content-type,connect-protocol-version,connect-timeout-ms,x-user-agent");
        res.setHeader("access-control-allow-methods", "POST,OPTIONS");
        if (req.method === "OPTIONS") { res.writeHead(204).end(); return; }
        const abort = new AbortController();
        res.on("close", () => abort.abort());
        try {
            const chunks: Buffer[] = [];
            for await (const chunk of req) chunks.push(chunk);
            const handler = router.handlers.find(handler => handler.requestPath === req.url);
            if (!handler) { res.writeHead(404).end(); return; }
            const headers = new Headers();
            for (const [name, value] of Object.entries(req.headers)) if (value) headers.set(name, Array.isArray(value) ? value.join(", ") : value);
            const response = await createFetchHandler(handler)(new Request(`http://127.0.0.1${req.url}`, {method:"POST", headers, body: Buffer.concat(chunks), signal:abort.signal}));
            res.writeHead(response.status, Object.fromEntries(response.headers));
            if (response.body) for await (const chunk of response.body) { if (!res.write(chunk)) await once(res,"drain",{signal:abort.signal}); }
            res.end();
        } catch (error) { if (!abort.signal.aborted) res.destroy(error instanceof Error ? error : new Error(String(error))); }
    });
    server.listen(0, "127.0.0.1");
    await once(server, "listening");
    const address = server.address();
    if (!address || typeof address === "string") throw new Error("Missing test server address");
    const endpoint = {url:`http://127.0.0.1:${address.port}`,token:"test-client",launchId:"fixture"};
    config = {...config, responses: {__clientEndpoint: endpoint, ...config.responses}};
    page.once("close", () => { server.closeAllConnections(); server.close(); });
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

        window.__releashRepositoryPaths = () => (cfg.responses.repository_paths ?? []) as string[];
        let workspaceSnapshot: WorkspaceListSnapshotDto | null = null;

		async function executeCommand(
			cmd: string,
			args: Record<string, unknown> = {},
		): Promise<unknown> {
            invocations.push({ cmd, args });
            if (cmd === "get_client_endpoint") return cfg.responses.__clientEndpoint;
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

            if (cmd === "get_workspaces" && !(cmd in cfg.responses)) {
                return workspaceSnapshot ?? { generation: 0, status: { loaded: false, error: null, state: "loading" }, repositories: [] };
            }
            if (cmd === "refresh_workspaces" && !(cmd in cfg.responses)) {
                if (args.worktreePath) {
                    const tree = workspaceSnapshot?.repositories.flatMap(repo => repo.worktrees).find(tree => tree.path === args.worktreePath);
                    if (tree) {
                        const worktreePath = tree.path;
                        tree.snapshot = await executeCommand("list_workspace_worktree_nodes", { worktreePath }) as typeof tree.snapshot;
                        tree.workflowHistory = await executeCommand("list_workspace_workflow_history", { worktreePath }) as typeof tree.workflowHistory;
                        tree.status = { loaded: true, error: null, state: tree.snapshot?.nodes.length ? "ready" : "empty" };
                    }
                    return workspaceSnapshot;
                }
                const paths = (cfg.responses.repository_paths ?? []) as string[];
                const repositories = await Promise.all(paths.map(async (path) => {
                    const result = await executeCommand("list_branches_with_status_snapshot", { repoPath: path }) as { worktree_display_groups: { working_areas: Record<string, unknown>[] } };
                    const prs = await executeCommand("get_cached_pr_status", { repoPath: path }) as { open_prs: Record<string, { number: number; url: string }>; merged_branches: string[] };
                    const branches = result.worktree_display_groups.working_areas.map((branch) => {
                        const pr = prs.open_prs[branch.name as string];
                        return { ...branch, is_merged: branch.is_merged || (!pr && prs.merged_branches.includes(branch.name as string)), has_pr: Boolean(pr), pr_number: pr?.number ?? null, pr_url: pr?.url ?? null };
                    });
                    const worktrees = await Promise.all(branches.filter(branch => branch.worktree_path).map(async (branch) => {
                        const worktreePath = branch.worktree_path as string;
                        const snapshot = await executeCommand("list_workspace_worktree_nodes", { worktreePath }) as { nodes: unknown[] };
                        return { path: worktreePath, status: { loaded: true, error: null, state: snapshot.nodes.length ? "ready" : "empty" }, snapshot, workflowHistory: await executeCommand("list_workspace_workflow_history", { worktreePath }) };
                    }));
                    return { path, status: { loaded: true, error: null, state: branches.length ? "ready" : "empty" }, branches, worktrees };
                }));
                workspaceSnapshot = { generation: invocations.length, status: { loaded: true, error: null, state: repositories.length ? "ready" : "empty" }, repositories } as WorkspaceListSnapshotDto;
                return workspaceSnapshot;
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
			if (cmd === "validate_daemon_connection") return null;
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
						"get_daemon_status", "retry_daemon", "quit_desktop", "restart_desktop", "validate_daemon_connection", "get_login_item_status", "open_login_item_settings", "install_cli", "set_login_item_enabled", "check_desktop_update", "install_desktop_update", "apply_desktop_settings",
						"complete_desktop_restoration", "fail_desktop_restoration", "get_client_endpoint",
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
	return { clientRequests, push };
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
