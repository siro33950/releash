import { create, toBinary } from "@bufbuild/protobuf";
import { invoke } from "@tauri-apps/api/core";
import {
	type Envelope,
	EnvelopeSchema,
	type OperationReference,
	PushSchema,
} from "@/generated/client_pb";
import { CLIENT_TRANSPORT as policy } from "@/generated/client_transport";
import type {
	ClientCommandArgs,
	ClientCommandResults,
	ClientPushPayloads,
} from "@/generated/client_types";
import {
	ClientStreamDecoder,
	createClientCommand,
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
export type ClientCommand = keyof ClientCommandArgs;
export type EmptyClientCommand = {
	[K in ClientCommand]: Record<string, never> extends ClientCommandArgs[K]
		? K
		: never;
}[ClientCommand];
export type ClientRequestState = "not_sent" | "unknown";
export interface ClientStatus {
	connected: boolean;
	message: string | null;
	operations: Array<{
		id: string;
		command: ClientCommand;
		state: ClientRequestState;
		expired: boolean;
	}>;
}
export class ClientTransportError extends Error {
	constructor(
		readonly requestId: string,
		readonly state: ClientRequestState,
	) {
		super(
			state === "not_sent"
				? "接続が回復しなかったため、この要求は送信されていません（未実行）。"
				: "操作結果を確認できません。元の操作の結果を確認しています。再実行は行っていません。",
		);
	}
}
interface RequestOptions {
	onUncertain?: (error: ClientTransportError) => void;
}
interface PendingRequest extends RequestOptions {
	id: string;
	command: ClientCommand;
	args: Record<string, unknown>;
	deadline: number;
	startedAt: number;
	watchId?: number;
	fingerprint: Uint8Array;
	orderingTarget: Uint8Array;
	sent: boolean;
	instanceId: string;
	uncertain: boolean;
	expired: boolean;
	userRetry: boolean;
	queryId: string | null;
	watchResult?: number;
	successors: OperationReference[];
	resolve(value: unknown): void;
	reject(reason: unknown): void;
}
let connection: Promise<WebSocket> | null = null;
let desktopSettingsUpdate: Promise<void> = Promise.resolve();
let connectedSocket: WebSocket | null = null;
let instanceId = "";
let hasConnected = false;
let refreshOnConnect = false;
let nextWatchId = 0;
const refreshListeners = new Set<() => void>();
let connectionMessage: string | null = null;
let status: ClientStatus = { connected: false, message: null, operations: [] };
const statusListeners = new Set<() => void>();
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
	event: keyof ClientPushPayloads;
	listener: (payload: never) => void;
	onReconnect: () => void;
}>();
const pending = new Map<string, PendingRequest>();
const notSentOperations = new Map<string, ClientCommand>();
const watches = new Map<
	number,
	{
		command: "start_watching" | "start_git_dir_watching";
		args: Record<string, unknown>;
		remoteId: number;
		instanceId: string;
		requestId: string;
		refreshOnReady: boolean;
		onReady?: (id: number) => void;
		onError?: (error: unknown) => void;
	}
>();
let reconnectTimer: ReturnType<typeof setTimeout> | null = null;
let tickTimer: ReturnType<typeof setInterval> | null = null;
let lastTick = 0;
let timing: {
	heartbeatIntervalMs: number;
	heartbeatTimeoutMs: number;
	connectTimeoutMs: number;
	reconnectIntervalMs: number;
	sleepGapMs: number;
	tickIntervalMs: number;
} = policy;
let deadlines: Partial<Record<ClientCommand, number>> = {};
let actions: Partial<
	Record<
		ClientCommand,
		{
			waitsForResult: boolean;
			pollsResult: boolean;
			disconnect: readonly string[];
		}
	>
> = {};
function requestPolicy(command: ClientCommand) {
	return actions[command] ?? policy.commands[command];
}
let heartbeat: { nonce: string; sentAt: number } | null = null;
let lastHeartbeat = 0;
let failConnection: (() => void) | null = null;

function publishStatus() {
	status = {
		connected: connectedSocket !== null,
		message: connectionMessage,
		operations: [...pending.values()]
			.filter((request) => request.uncertain || !request.sent)
			.map((request) => ({
				id: request.id,
				command: request.command,
				state: request.uncertain ? ("unknown" as const) : ("not_sent" as const),
				expired: request.expired,
			}))
			.concat(
				[...notSentOperations].map(([id, command]) => ({
					id,
					command,
					state: "not_sent" as const,
					expired: true,
				})),
			),
	};
	for (const listener of statusListeners) listener();
}
export function getClientStatus() {
	return status;
}
export function dismissClientOperation(id: string) {
	notSentOperations.delete(id);
	publishStatus();
}
export function subscribeClientStatus(listener: () => void) {
	statusListeners.add(listener);
	void connect().catch(() => {});
	return () => {
		statusListeners.delete(listener);
	};
}
function refreshState() {
	for (const listener of refreshListeners) listener();
	for (const entry of listeners) entry.onReconnect();
}
function sendControl(body: Envelope["body"]) {
	connectedSocket?.send(
		toBinary(EnvelopeSchema, create(EnvelopeSchema, { body })),
	);
}
function probe() {
	heartbeat = { nonce: crypto.randomUUID(), sentAt: Date.now() };
	lastHeartbeat = heartbeat.sentAt;
	sendControl({
		case: "heartbeat",
		value: { $typeName: "releash.client.v1.Heartbeat", nonce: heartbeat.nonce },
	});
}
function predecessors(request: PendingRequest) {
	const references = [];
	for (const previous of pending.values()) {
		if (previous === request) break;
		references.push({
			$typeName: "releash.client.v1.OperationReference" as const,
			requestId: previous.id,
			command: previous.command,
			uncertain: previous.uncertain,
			fingerprint: previous.fingerprint,
			orderingTarget: previous.orderingTarget,
		});
	}
	return references;
}
function query(request: PendingRequest) {
	if (!connectedSocket) return;
	request.queryId = crypto.randomUUID();
	sendControl({
		case: "operationQuery",
		value: {
			$typeName: "releash.client.v1.OperationQuery",
			queryId: request.queryId,
			requestId: request.id,
			instanceId: request.instanceId || instanceId,
			request: createClientCommand(
				request.id,
				request.command,
				request.args,
				request.instanceId || instanceId,
				request.sent || request.uncertain,
				predecessors(request),
				request.deadline,
				request.userRetry,
				request.successors,
			),
			sent: request.sent || request.uncertain,
			expired: request.expired,
		},
	});
}
export function retryClientOperation(id: string) {
	const request = pending.get(id);
	if (request && (request.sent || request.uncertain)) {
		request.userRetry = true;
		if (request.expired) {
			request.expired = false;
			request.startedAt = Date.now();
			request.deadline =
				request.startedAt +
				(deadlines[request.command] ??
					policy.commands[request.command].deadlineMs);
		}
		query(request);
	}
	void connect().catch(() => {});
}
function startClock() {
	if (tickTimer) return;
	lastTick = Date.now();
	tickTimer = setInterval(() => {
		const now = Date.now();
		const resumed = now - lastTick > timing.sleepGapMs;
		lastTick = now;
		for (const request of pending.values()) {
			if (!request.expired && now >= request.deadline) {
				request.expired = true;
				if (!connectedSocket) refreshOnConnect = true;
				request.uncertain ||= request.sent;
				if (request.uncertain)
					request.onUncertain?.(
						new ClientTransportError(request.id, "unknown"),
					);
				if (
					!request.uncertain ||
					!requestPolicy(request.command).waitsForResult
				) {
					request.reject(
						new ClientTransportError(
							request.id,
							request.uncertain ? "unknown" : "not_sent",
						),
					);
				}
				if (!request.uncertain) {
					notSentOperations.set(request.id, request.command);
					pending.delete(request.id);
				}

				publishStatus();
			}
		}
		if (!connectedSocket) return;
		if (resumed) {
			probe();
			return;
		}
		if (heartbeat && now - heartbeat.sentAt >= timing.heartbeatTimeoutMs) {
			connectionMessage = "通信状態を確認できません。再接続しています。";
			failConnection?.();
			return;
		}
		if (!heartbeat && now - lastHeartbeat >= timing.heartbeatIntervalMs) {
			probe();
			for (const request of pending.values())
				if (
					(request.uncertain || !request.sent) &&
					requestPolicy(request.command).pollsResult
				)
					query(request);
		}
	}, timing.tickIntervalMs);
}
function scheduleReconnect() {
	if (
		reconnectTimer ||
		(!pending.size &&
			!listeners.size &&
			!connectionListeners.size &&
			!statusListeners.size &&
			!refreshListeners.size)
	)
		return;
	reconnectTimer = setTimeout(() => {
		reconnectTimer = null;
		if (
			!pending.size &&
			!listeners.size &&
			!connectionListeners.size &&
			!statusListeners.size &&
			!refreshListeners.size
		)
			return;
		void connect().catch(() => {});
	}, timing.reconnectIntervalMs);
}
function sendRequest(request: PendingRequest, authorized = false) {
	if (!connectedSocket || Date.now() >= request.deadline) return;
	try {
		if (!authorized) {
			query(request);
			return;
		}
		const bytes = encodeClientCommand(
			request.id,
			request.command,
			request.args,
			request.instanceId || instanceId,
			request.sent || request.uncertain,
			predecessors(request),
			request.deadline,
			request.userRetry,
			request.successors,
		);
		request.instanceId ||= instanceId;
		connectedSocket.send(bytes);
		request.sent = true;
	} catch (error) {
		if (!request.sent) {
			pending.delete(request.id);
			request.reject(error);
		} else request.uncertain = true;
	}
}
function restoreWatches() {
	for (const [id, watch] of watches) {
		if (pending.has(watch.requestId)) continue;
		const identity = {
			id:
				watch.instanceId === instanceId ? watch.requestId : crypto.randomUUID(),
			instanceId: watch.instanceId === instanceId ? watch.instanceId : "",
		};
		watch.requestId = identity.id;
		void requestCommand(watch.command, watch.args, id, identity).catch(
			(error) => watch.onError?.(error),
		);
	}
}
function connect(): Promise<WebSocket> {
	if (connection) return connection;
	startClock();
	const attempt = invoke<ClientEndpoint | null>("get_client_endpoint")
		.then((endpoint) => {
			if (!endpoint) throw new Error("Client endpoint is unavailable");
			return new Promise<WebSocket>((resolve, reject) => {
				const socket = new WebSocket(endpoint.url, [endpoint.authSubprotocol]);
				socket.binaryType = "arraybuffer";
				const timeout = setTimeout(() => fail(), timing.connectTimeoutMs);
				let closed = false;
				function fail() {
					if (closed) return;
					closed = true;
					clearTimeout(timeout);
					if (connection === attempt) connection = null;
					reject(new Error("Client WebSocket connection closed"));
					connectedSocket = null;
					heartbeat = null;
					streams.clear();
					for (const request of pending.values()) {
						if (!request.sent) continue;
						if (
							requestPolicy(request.command).disconnect[
								Number(request.expired)
							] === "release"
						) {
							request.reject(new ClientTransportError(request.id, "unknown"));
							pending.delete(request.id);
						} else {
							request.uncertain = true;
							request.onUncertain?.(
								new ClientTransportError(request.id, "unknown"),
							);
						}
					}
					connectionMessage ||= "接続が切れています。再接続しています。";
					publishStatus();
					for (const listener of connectionListeners) listener(false);
					socket.close();
					scheduleReconnect();
				}
				failConnection = fail;
				socket.onopen = () => {
					if (!closed)
						socket.send(
							toBinary(
								EnvelopeSchema,
								create(EnvelopeSchema, { body: { case: "hello", value: {} } }),
							),
						);
				};
				socket.onerror = fail;
				socket.onclose = fail;
				socket.onmessage = async (event) => {
					if (closed) return;
					try {
						const { body } = decodeClientEnvelope(event.data);
						if (
							(body.case === "hello" || body.case === "response") &&
							body.value.desktopSettings
						) {
							const settings = body.value.desktopSettings;
							const apply = () =>
								invoke<void>("apply_desktop_settings", { settings });
							desktopSettingsUpdate = desktopSettingsUpdate.then(apply, apply);
							await desktopSettingsUpdate;
							if (closed) return;
						}
						if (body.case === "hello") {
							if (!body.value.instanceId)
								throw new Error("Missing backend identity");
							clearTimeout(timeout);
							if (body.value.heartbeatIntervalMs > 0n) {
								timing = {
									heartbeatIntervalMs: Number(body.value.heartbeatIntervalMs),
									heartbeatTimeoutMs: Number(body.value.heartbeatTimeoutMs),
									connectTimeoutMs: Number(body.value.connectTimeoutMs),
									reconnectIntervalMs: Number(body.value.reconnectIntervalMs),
									sleepGapMs: Number(body.value.sleepGapMs),
									tickIntervalMs: Number(body.value.tickIntervalMs),
								};
								deadlines = Object.fromEntries(
									Object.entries(body.value.deadlinesMs).map(
										([name, value]) => [name, Number(value)],
									),
								);
							}
							if (tickTimer) clearInterval(tickTimer);
							tickTimer = null;
							startClock();
							for (const request of pending.values())
								request.deadline =
									request.startedAt +
									(deadlines[request.command] ??
										policy.commands[request.command].deadlineMs);
							actions = Object.fromEntries(
								Object.entries(body.value.policies).map(([command, value]) => [
									command,
									{
										...value,
										disconnect: [value.disconnect, value.expiredDisconnect],
									},
								]),
							);
							instanceId = body.value.instanceId;
							connectedSocket = socket;
							connectionMessage = null;
							lastHeartbeat = Date.now();
							resolve(socket);
							for (const request of pending.values()) query(request);
							publishStatus();
							for (const listener of connectionListeners) listener(true);
							restoreWatches();
							if (hasConnected || refreshOnConnect) refreshState();
							refreshOnConnect = false;
							hasConnected = true;
							return;
						}
						if (body.case === "heartbeat") {
							if (heartbeat?.nonce === body.value.nonce) heartbeat = null;
							return;
						}
						if (body.case === "operationStatus") {
							const request = pending.get(body.value.requestId);
							if (!request) return;
							if (
								body.value.state === "watch_active" &&
								request.watchResult !== undefined
							) {
								const watch =
									request.watchId === undefined
										? undefined
										: watches.get(request.watchId);
								if (watch?.requestId === request.id) {
									watch.remoteId = request.watchResult;
									watch.instanceId = instanceId;
									watch.onReady?.(request.watchId as number);
									request.resolve(request.watchId);
									pending.delete(request.id);
									const refresh =
										watch.refreshOnReady ||
										request.uncertain ||
										request.expired;
									watch.refreshOnReady = false;
									if (refresh) refreshState();
								} else {
									socket.send(encodeClientRequestAck(request.id, true));
									pending.delete(request.id);
								}
							} else if (
								body.value.state === "watch_released" &&
								request.watchId !== undefined
							) {
								request.uncertain = request.sent;
								request.watchResult = undefined;
								const watch = watches.get(request.watchId);
								if (watch?.requestId === request.id) {
									watch.remoteId = 0;
									watch.refreshOnReady = true;
									watch.instanceId = "";
									watch.requestId = crypto.randomUUID();
									void requestCommand(
										watch.command,
										watch.args,
										request.watchId,
										{ id: watch.requestId, instanceId: "" },
									).catch(watch.onError);
								}
							} else if (body.value.state === "ready") {
								if (!request.queryId || body.value.queryId !== request.queryId)
									return;
								request.queryId = null;
								request.fingerprint = body.value.fingerprint;
								request.orderingTarget = body.value.orderingTarget;
								sendRequest(request, true);
							} else if (body.value.state === "bound") {
								pending.delete(request.id);
								const original = pending.get(body.value.operationId);
								if (original) {
									const resolve = original.resolve;
									const reject = original.reject;
									const onUncertain = original.onUncertain;
									original.onUncertain = (error) => {
										onUncertain?.(error);
										request.onUncertain?.(error);
									};
									if (original.uncertain)
										request.onUncertain?.(
											new ClientTransportError(original.id, "unknown"),
										);
									original.resolve = (value) => {
										resolve(value);
										request.resolve(value);
									};
									original.reject = (error) => {
										reject(error);
										request.reject(error);
									};
								} else
									request.reject(
										new ClientTransportError(body.value.operationId, "unknown"),
									);
								if (original) query(original);
							} else if (
								body.value.state === "not_sent" ||
								body.value.state === "disconnected"
							) {
								request.reject(
									new ClientTransportError(
										request.id,
										body.value.state === "not_sent" ? "not_sent" : "unknown",
									),
								);
								if (body.value.state === "not_sent")
									notSentOperations.set(request.id, request.command);
								pending.delete(request.id);
							} else if (body.value.state === "unknown") {
								request.uncertain = true;
								request.onUncertain?.(
									new ClientTransportError(request.id, "unknown"),
								);
							}
							publishStatus();
							return;
						}
						if (body.case === "pushResync") {
							refreshState();
							return;
						}
						if (body.case === "streamClosed") {
							const entry = streams.get(body.value.attachmentId);
							streams.delete(body.value.attachmentId);
							entry?.onClosed();
							return;
						}
						if (body.case === "push") {
							const field = PushSchema.fields.find(
								(field) => field.localName === body.value.event.case,
							);
							if (!field) throw new Error("Missing push event");
							const name = field.name.replace(
								/_/g,
								"-",
							) as keyof ClientPushPayloads;
							let payload = decodeClientPush(body.value, name);
							if (name === "file-change") {
								const change = payload as ClientPushPayloads["file-change"];
								for (const [id, watch] of watches)
									if (
										watch.instanceId === instanceId &&
										watch.remoteId === change.watcher_id
									) {
										payload = { ...change, watcher_id: id };
										break;
									}
							}
							for (const entry of listeners)
								if (entry.event === name) entry.listener(payload as never);
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
							if (request.watchId !== undefined) {
								request.watchResult = Number(value);
								const watch = watches.get(request.watchId);
								const released = watch?.requestId !== request.id;
								socket.send(
									encodeClientRequestAck(request.id, released, !released),
								);
								if (released && !watch) pending.delete(request.id);
								return;
							}
							request.resolve(value);
						} else throw new Error("Invalid command response");
						if (requestPolicy(request.command).waitsForResult)
							socket.send(
								encodeClientRequestAck(
									request.id,
									request.watchId !== undefined &&
										!watches.has(request.watchId),
								),
							);
						for (const earlier of pending.values()) {
							if (earlier === request) break;
							if (
								request.orderingTarget.length &&
								earlier.orderingTarget.toString() ===
									request.orderingTarget.toString()
							) {
								earlier.successors = [
									{
										$typeName: "releash.client.v1.OperationReference",
										requestId: request.id,
										command: request.command,
										uncertain: false,
										fingerprint: request.fingerprint,
										orderingTarget: request.orderingTarget,
									},
								];
							}
						}
						pending.delete(request.id);
						for (const queued of pending.values())
							if (!queued.sent) sendRequest(queued);
						publishStatus();
						if (request.uncertain || request.expired) refreshState();
					} catch (error) {
						console.error("Client WebSocket frame failed:", error);
						for (const request of pending.values()) {
							if (request.sent) {
								request.expired = true;
								request.uncertain = true;
								if (!requestPolicy(request.command).waitsForResult)
									request.reject(
										new ClientTransportError(request.id, "unknown"),
									);
							}
						}
						fail();
					}
				};
			});
		})
		.catch((error) => {
			if (connection === attempt) connection = null;
			connectionMessage ||= "接続情報を取得できません。再接続しています。";
			publishStatus();
			scheduleReconnect();
			throw error;
		});
	connection = attempt;
	return attempt;
}
function requestCommand(
	command: ClientCommand,
	args: Record<string, unknown>,
	watchId?: number,
	identity?: { id: string; instanceId: string; sent?: boolean },
	options?: RequestOptions,
): Promise<unknown> {
	startClock();
	const requestId = identity?.id ?? crypto.randomUUID();
	const promise = new Promise((resolve, reject) => {
		const request: PendingRequest = {
			id: requestId,
			command,
			args,
			startedAt: Date.now(),
			watchId,
			fingerprint: new Uint8Array(),
			orderingTarget: new Uint8Array(),
			deadline:
				Date.now() +
				(deadlines[command] ?? policy.commands[command].deadlineMs),
			sent: identity?.sent ?? Boolean(identity?.instanceId),
			instanceId: identity?.instanceId ?? "",
			uncertain: false,
			expired: false,
			userRetry: false,
			queryId: null,
			onUncertain: options?.onUncertain,
			successors: [],
			resolve,
			reject,
		};
		pending.set(requestId, request);
		if (request.sent) query(request);
		else sendRequest(request);
		void connect().catch(() => {});
	});
	return promise;
}

export function watchClient(
	command: "start_watching" | "start_git_dir_watching",
	args: Record<string, unknown>,
	onReady: (id: number) => void,
	onError: (error: unknown) => void = console.error,
): () => void {
	const id = --nextWatchId;
	const requestId = crypto.randomUUID();
	watches.set(id, {
		command,
		args,
		remoteId: 0,
		instanceId: "",
		requestId,
		refreshOnReady: false,
		onReady,
		onError,
	});
	void requestCommand(command, args, id, {
		id: requestId,
		instanceId: "",
	}).catch(onError);
	return () => {
		const watch = watches.get(id);
		watches.delete(id);
		if (watch?.remoteId)
			void requestCommand(
				"stop_watching",
				{ watcherId: watch.remoteId },
				undefined,
				{ id: crypto.randomUUID(), instanceId: watch.instanceId, sent: false },
			).catch(onError);
	};
}

export function onClientRefresh(listener: () => void) {
	refreshListeners.add(listener);
	void connect().catch(() => {});
	return () => {
		refreshListeners.delete(listener);
	};
}

export function invokeClient<K extends ClientCommand>(
	command: K,
	args?: ClientCommandArgs[K],
	options?: RequestOptions,
): Promise<ClientCommandResults[K]> {
	const input: Record<string, unknown> = args ?? {};
	if (command === "stop_watching") {
		const id = Number(input.watcherId);
		const watch = watches.get(id);
		watches.delete(id);
		if (watch)
			return requestCommand(
				command,
				{ ...input, watcherId: watch.remoteId },
				undefined,
				{ id: crypto.randomUUID(), instanceId: watch.instanceId, sent: false },
				options,
			) as Promise<ClientCommandResults[K]>;
	}
	if (command === "start_watching" || command === "start_git_dir_watching") {
		return new Promise((resolve, reject) => {
			let received = false;
			const stop = watchClient(
				command,
				input,
				(id) => {
					received = true;
					resolve(id as ClientCommandResults[K]);
				},
				(error) => {
					if (!received) stop();
					reject(error);
				},
			);
		});
	}
	return requestCommand(
		command,
		input,
		undefined,
		undefined,
		options,
	) as Promise<ClientCommandResults[K]>;
}
export async function listenClient<K extends keyof ClientPushPayloads>(
	event: K,
	listener: (event: { payload: ClientPushPayloads[K] }) => void,
	onReconnect: () => void = () => {},
): Promise<() => void> {
	const entry = {
		event,
		listener: (payload: never) => listener({ payload }),
		onReconnect,
	};
	listeners.add(entry);
	void connect().catch(() => {});
	return () => {
		listeners.delete(entry);
	};
}
export function onClientConnection(
	listener: (connected: boolean) => void,
): () => void {
	connectionListeners.add(listener);
	void connect().catch(() => {});
	return () => {
		connectionListeners.delete(listener);
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
	connectedSocket?.send(encodeClientAck(attachmentId, 0, sequence));
}

window.addEventListener("pagehide", () => {
	if (tickTimer) clearInterval(tickTimer);
	if (reconnectTimer) clearTimeout(reconnectTimer);
	tickTimer = null;
	reconnectTimer = null;
	for (const request of pending.values())
		request.reject(
			new ClientTransportError(
				request.id,
				request.sent || request.uncertain ? "unknown" : "not_sent",
			),
		);
	pending.clear();
	notSentOperations.clear();
	listeners.clear();
	refreshListeners.clear();
	hasConnected = false;
	refreshOnConnect = false;
	nextWatchId = 0;
	connectionListeners.clear();
	statusListeners.clear();
	watches.clear();
	connectedSocket?.close();
	connectedSocket = null;
	connection = null;
	heartbeat = null;
	instanceId = "";
	timing = policy;
	deadlines = {};
	actions = {};
	connectionMessage = null;
	publishStatus();
});
