import { fromJson, toJson } from "@bufbuild/protobuf";
import {
	type Client,
	Code,
	ConnectError,
	createClient,
} from "@connectrpc/connect";
import { createLinkedAbortController } from "@connectrpc/connect/protocol";
import { createConnectTransport } from "@connectrpc/connect-web";
import { invoke } from "@tauri-apps/api/core";
import {
	AttachTerminalSurfaceRequestSchema,
	ClientService,
	CommandErrorSchema,
	StartWatchingRequestSchema,
	type StatePayload,
	StatePayloadSchema,
	type StateVersion,
	type TerminalEvent,
} from "@/generated/client_pb";
import type {
	ClientCommandArgs,
	ClientPushPayloads,
} from "@/generated/client_types";
import { clientJson } from "./clientJson";
import { decodeClientPush, decodeTerminalEvent } from "./clientProtocol";
import type { TerminalSurfaceStreamItem } from "./terminalSurfaceStream";

export {
	type ClientCommand,
	type EmptyClientCommand,
	invokeClient,
} from "@/generated/client_commands";

type Endpoint = { url: string; token: string; launchId: string };
type Session = {
	client: Client<typeof ClientService>;
	endpoint: Endpoint;
	attachmentId: string;
};
let session: Promise<Session> | null = null;
let current: Session | null = null;
let connectionAbort = new AbortController();
let pushTask: Promise<void> | null = null;
let stopped = false;
type PushSubscription = { client: Client<typeof ClientService>; id: string };
let activeSubscription: PushSubscription | null = null;
const watchers = new Set<
	(subscription: PushSubscription, refreshOnReady?: boolean) => Promise<void>
>();
const refreshListeners = new Set<() => void>();
const connectionListeners = new Set<(connected: boolean) => void>();
const listeners = new Set<{
	event: keyof ClientPushPayloads;
	listener: (payload: never) => void;
	onReconnect: () => void;
}>();

async function open(abort: AbortController): Promise<Session> {
	const attachmentId = crypto.randomUUID();
	const endpoint = await invoke<Endpoint>("get_client_endpoint", {
		attachmentId,
	});
	abort.signal.throwIfAborted();
	const client = createClient(
		ClientService,
		createConnectTransport({
			baseUrl: endpoint.url,
			useBinaryFormat: true,
			defaultTimeoutMs: 120_000,
			interceptors: [
				(next) => async (request) => {
					request.header.set("Authorization", `Bearer ${endpoint.token}`);
					const response = await next(request);
					if (
						response.header.get("releash-desktop-settings-changed") === "true"
					)
						await applyClientDesktopSettings(client).catch((error) => {
							console.error("Failed to synchronize desktop settings", error);
						});
					return response;
				},
			],
			fetch: (input, init) => {
				const request = new Request(input, init);
				return fetch(request, {
					signal: createLinkedAbortController(request.signal, abort.signal)
						.signal,
				});
			},
		}),
	);
	const info = await client.getServerInfo({});
	await invoke("validate_daemon_connection", {
		launchId: info.launchId,
		release: info.release,
	});
	if (info.desktopSettings)
		await invoke("apply_desktop_settings", { settings: info.desktopSettings });
	abort.signal.throwIfAborted();
	const result = { client, endpoint, attachmentId };
	current = result;
	for (const listener of connectionListeners) listener(true);
	return result;
}

export async function getClient(): Promise<Client<typeof ClientService>> {
	stopped = false;
	if (connectionAbort.signal.aborted) connectionAbort = new AbortController();
	if (!session) {
		const pending = open(connectionAbort);
		session = pending;
		void pending.catch(() => {
			if (session === pending) session = null;
		});
	}
	return (await session).client;
}

export function refreshClient(failedClient?: Client<typeof ClientService>) {
	if (failedClient && current?.client !== failedClient) return;
	const previous = current;
	activeSubscription = null;
	current = null;
	session = null;
	connectionAbort.abort();
	connectionAbort = new AbortController();
	if (previous) for (const listener of connectionListeners) listener(false);
	ensurePush();
}

function refreshState() {
	for (const listener of refreshListeners) listener();
	for (const entry of listeners) entry.onReconnect();
}

function ensurePush() {
	if (pushTask || stopped) return;
	pushTask = (async () => {
		while (!stopped) {
			let client: Client<typeof ClientService> | undefined;
			try {
				client = await getClient();
				const subscription = { client, id: crypto.randomUUID() };
				let initial = true;
				for await (const push of client.subscribePush(
					{ subscriptionId: subscription.id },
					{ timeoutMs: 0 },
				)) {
					if (push.event.case === "resync") {
						if (initial) {
							initial = false;
							activeSubscription = subscription;
							void Promise.all(
								[...watchers].map((watcher) => watcher(subscription, false)),
							)
								.then(() => {
									if (activeSubscription === subscription) refreshState();
								})
								.catch((error) => {
									console.debug("Client watcher restoration failed", error);
									refreshClient(subscription.client);
								});
							continue;
						}
						if (activeSubscription === subscription) refreshState();
						continue;
					}
					const matching = [...listeners].filter(
						(entry) =>
							push.event.case ===
							entry.event.replace(/-([a-z])/g, (_, letter: string) =>
								letter.toUpperCase(),
							),
					);
					if (matching.length) {
						const payload = decodeClientPush(push, matching[0].event);
						for (const entry of matching) entry.listener(payload as never);
					}
				}
			} catch (error) {
				if (stopped) break;
				console.debug("Client subscription ended", error);
			}
			if (!stopped) {
				refreshClient(client);
				await new Promise((resolve) => setTimeout(resolve, 1000));
			}
		}
	})().finally(() => {
		pushTask = null;
	});
}

export function onClientRefresh(listener: () => void) {
	stopped = false;
	refreshListeners.add(listener);
	ensurePush();
	return () => {
		refreshListeners.delete(listener);
	};
}

export async function listenClient<K extends keyof ClientPushPayloads>(
	event: K,
	listener: (event: { payload: ClientPushPayloads[K] }) => void,
	onReconnect = () => {},
) {
	const entry = {
		event,
		listener: (payload: never) => listener({ payload }),
		onReconnect,
	};
	stopped = false;
	listeners.add(entry);
	ensurePush();
	return () => {
		listeners.delete(entry);
	};
}

export type StateValues = {
	failures: import("@/generated/client_types").FailureRecords;
	"repository-paths": string[];
	workspaces: import("@/generated/client_types").WorkspaceListSnapshotDto;
	selection: import("@/generated/client_types").WorkspaceTreeSelectionSnapshotDto;
	"node-detail":
		| import("@/generated/client_types").WorkspaceNodeDetailDto
		| null;
	"agent-session":
		| import("@/generated/client_types").AgentSessionItemDto
		| null;
	"session-node": string | null;
	"session-history": import("@/generated/client_types").AgentSessionHistoryPageDto;
	providers: import("@/generated/client_types").AgentSessionProviderDto[];
	branches: import("@/generated/client_types").BranchDto[];
	"branch-base": string | null;
	"branch-status": import("@/generated/client_types").RepositoryBranchCardsSnapshotDto;
	"current-branch": string;
	issues: import("@/generated/client_types").IssueInfoDto[];
	worktrees: import("@/generated/client_types").WorktreeEntryDto[];
	"repository-root": string;
	"startup-repository": string;
	"workspace-state":
		| import("@/generated/client_types").WorkspaceStateDto
		| null;
};
export type StateTarget<K extends keyof StateValues> =
	| K
	| { kind: K; args: string[] };
function stateTargetKey(kind: string, args: string[]) {
	return JSON.stringify([kind, args]);
}
type StateEntry = {
	kind: string;
	args: string[];
	receivers: Set<(value: never) => void>;
	errors: Set<(error: unknown) => void>;
	version?: StateVersion;
	value?: { current: unknown };
};
type StateStream = { client: Client<typeof ClientService>; id: string };
const STATE_SILENCE_MS = 30_000;
const IDLE = "idle";
const states = new Map<string, StateEntry>();
let stateStream: StateStream | null = null;
let stateAbort: AbortController | null = null;
let stateTask: Promise<void> | null = null;

function decodeState(payload: StatePayload | undefined) {
	if (!payload?.value.case) return;
	const field = StatePayloadSchema.fields.find(
		(field) => field.localName === payload.value.case,
	);
	if (!field?.message) return;
	const json = toJson(StatePayloadSchema, payload) as Record<
		string,
		import("@bufbuild/protobuf").JsonValue
	>;
	return { current: clientJson(field.message, json[field.jsonName], false) };
}

function startState(stream: StateStream, target: string) {
	const entry = states.get(target);
	if (!entry) return;
	void stream.client
		.startStateSubscription({
			clientId: stream.id,
			target: entry.kind,
			args: entry.args,
			version: entry.version,
		})
		.catch((error) => {
			if (states.get(target) !== entry || stateStream !== stream) return;
			console.error("State subscription failed", error);
			for (const receiver of entry.errors) receiver(error);
		});
}

function ensureStateStream() {
	if (stateTask || stopped || !states.size) return;
	stateTask = (async () => {
		while (!stopped && states.size) {
			let client: Client<typeof ClientService> | undefined;
			const abort = new AbortController();
			stateAbort = abort;
			let silence: ReturnType<typeof setTimeout> | undefined;
			const alive = () => {
				clearTimeout(silence);
				silence = setTimeout(() => abort.abort(), STATE_SILENCE_MS);
			};
			try {
				client = await getClient();
				const stream = { client, id: crypto.randomUUID() };
				alive();
				for await (const event of client.openStateStream(
					{ clientId: stream.id },
					{ signal: abort.signal, timeoutMs: 0 },
				)) {
					alive();
					if (event.event.case === "ready") {
						stateStream = stream;
						for (const target of states.keys()) startState(stream, target);
						continue;
					}
					const entry = states.get(stateTargetKey(event.target, event.args));
					if (!entry) continue;
					entry.version = event.version;
					const value = decodeState(
						event.event.case === "snapshot"
							? event.event.value
							: event.event.case === "change"
								? event.event.value.payload
								: undefined,
					);
					if (!value) continue;
					entry.value = value;
					for (const receiver of entry.receivers)
						receiver(value.current as never);
				}
			} catch (error) {
				if (stopped) break;
				if (abort.signal.reason !== IDLE)
					console.debug("State stream ended", error);
			} finally {
				clearTimeout(silence);
				stateStream = null;
				if (stateAbort === abort) stateAbort = null;
			}
			if (abort.signal.reason === IDLE) continue;
			if (!stopped && states.size) {
				refreshClient(client);
				await new Promise((resolve) => setTimeout(resolve, 1000));
			}
		}
	})().finally(() => {
		stateTask = null;
		ensureStateStream();
	});
}

export function subscribeState<K extends keyof StateValues>(
	input: StateTarget<K>,
	onValue: (value: StateValues[K]) => void,
	onError?: (error: unknown) => void,
) {
	const kind = typeof input === "string" ? input : input.kind;
	const args = typeof input === "string" ? [] : input.args;
	const target = stateTargetKey(kind, args);
	const receiver = onValue as (value: never) => void;
	let entry = states.get(target);
	if (!entry) {
		entry = { kind, args, receivers: new Set(), errors: new Set() };
		states.set(target, entry);
		if (stateStream) startState(stateStream, target);
	} else if (entry.value) onValue(entry.value.current as StateValues[K]);
	entry.receivers.add(receiver);
	if (onError) entry.errors.add(onError);
	stopped = false;
	ensureStateStream();
	return () => {
		const current = states.get(target);
		if (current !== entry) return;
		current.receivers.delete(receiver);
		if (onError) current.errors.delete(onError);
		if (current.receivers.size) return;
		states.delete(target);
		if (!states.size) {
			stateStream = null;
			stateAbort?.abort(IDLE);
		} else if (stateStream)
			void stateStream.client
				.stopStateSubscription({ clientId: stateStream.id, target: kind, args })
				.catch((error) => console.debug("State unsubscribe failed", error));
	};
}

export function onClientConnection(listener: (connected: boolean) => void) {
	stopped = false;
	connectionListeners.add(listener);
	ensurePush();
	return () => {
		connectionListeners.delete(listener);
	};
}

export function watchClient(
	args: Record<string, unknown>,
	onReady: (id: number) => void,
	onError: (error: unknown) => void = console.error,
) {
	let abort = new AbortController();
	let release: (() => void) | undefined;
	let retry: ReturnType<typeof setTimeout> | undefined;
	let closed = false;
	const start = (subscription: PushSubscription, refreshOnReady = true) => {
		clearTimeout(retry);
		abort.abort();
		release?.();
		release = undefined;
		abort = new AbortController();
		const signal = abort.signal;
		const { client, id: subscriptionId } = subscription;
		const request = client.watchFiles(
			{
				subscriptionId,
				request: fromJson(
					StartWatchingRequestSchema,
					clientJson(
						StartWatchingRequestSchema,
						JSON.parse(JSON.stringify(args)),
						true,
					),
				),
			},
			{ signal },
		);
		return request
			.then((ready) => {
				const stop = () => {
					if (activeSubscription?.client !== client) return;
					void client
						.stopWatching({ watcherId: ready.value })
						.catch((error) => console.debug("Watcher cleanup failed", error));
				};
				if (closed || signal.aborted) {
					stop();
					return;
				}
				release = stop;
				onReady(Number(ready.value));
				if (refreshOnReady) refreshState();
			})
			.catch((error) => {
				if (signal.aborted || closed) return;
				onError(error);
				if (
					error instanceof ConnectError &&
					error.code === Code.ResourceExhausted
				) {
					retry = setTimeout(() => {
						if (!closed && activeSubscription === subscription)
							start(subscription);
					}, 1000);
				}
			});
	};
	watchers.add(start);
	if (activeSubscription) void start(activeSubscription, false);
	else {
		stopped = false;
		ensurePush();
	}
	return () => {
		closed = true;
		clearTimeout(retry);
		watchers.delete(start);
		release?.();
	};
}

type TerminalListener = {
	streamId: string;
	receive: (event: TerminalEvent) => void;
	close: (resynchronize: boolean, error?: unknown) => void;
};
type TerminalSubscription = {
	id: string;
	listeners: Map<string, TerminalListener>;
};
const terminalSubscriptions = new WeakMap<
	Client<typeof ClientService>,
	Promise<TerminalSubscription>
>();

function getTerminalSubscription(client: Client<typeof ClientService>) {
	const existing = terminalSubscriptions.get(client);
	if (existing) return existing;
	const abort = new AbortController();
	const pending = (async () => {
		const subscription: TerminalSubscription = {
			id: crypto.randomUUID(),
			listeners: new Map(),
		};
		const stream = client
			.subscribeTerminalSurfaces(
				{ subscriptionId: subscription.id },
				{ signal: abort.signal, timeoutMs: 0 },
			)
			[Symbol.asyncIterator]();
		const initial = await stream.next();
		if (initial.done || initial.value.event.case !== "ready")
			throw new Error("Terminal subscription closed before ready");
		void (async () => {
			let failure: unknown;
			try {
				for (;;) {
					const next = await stream.next();
					if (next.done) break;
					const { attachmentId, streamId, event } = next.value;
					const listener = subscription.listeners.get(attachmentId);
					if (listener?.streamId !== streamId) continue;
					if (event.case === "item") listener?.receive(event.value);
					else if (event.case === "closed") {
						subscription.listeners.delete(attachmentId);
						listener?.close(event.value.resynchronize);
					}
				}
			} catch (error) {
				failure = error;
			} finally {
				abort.abort();
				terminalSubscriptions.delete(client);
				for (const listener of subscription.listeners.values())
					listener.close(true, failure);
				subscription.listeners.clear();
			}
		})();
		return subscription;
	})();
	terminalSubscriptions.set(client, pending);
	void pending.catch(() => {
		abort.abort();
		terminalSubscriptions.delete(client);
	});
	return pending;
}

export async function attachClientStream(
	args: ClientCommandArgs["attach_terminal_surface"],
	listener: (item: TerminalSurfaceStreamItem) => void,
	onClosed: () => void,
): Promise<() => Promise<void>> {
	const client = await getClient();
	let subscription: TerminalSubscription | undefined;
	let active = true;
	const streamId = crypto.randomUUID();
	let released: Promise<void> | undefined;
	const release = () => {
		active = false;
		if (released) return released;
		const registered = subscription?.listeners.get(args.attachmentId);
		if (registered && registered.streamId !== streamId) {
			released = Promise.resolve();
			return released;
		}
		subscription?.listeners.delete(args.attachmentId);
		if (current?.client !== client) {
			released = Promise.resolve();
			return released;
		}
		released = client
			.detachTerminalSurface({ attachmentId: args.attachmentId })
			.then(() => {})
			.catch((error) => {
				if (
					current?.client !== client &&
					error instanceof ConnectError &&
					error.code === Code.Canceled
				)
					return;
				throw terminalError(error);
			});
		return released;
	};
	try {
		subscription = await getTerminalSubscription(client);
		let initialized = false;
		let resolveInitial!: () => void;
		let rejectInitial!: (error: unknown) => void;
		const initial = new Promise<void>((resolve, reject) => {
			resolveInitial = resolve;
			rejectInitial = reject;
		});
		const attached = client.attachTerminalSurface({
			subscriptionId: subscription.id,
			streamId,
			request: fromJson(
				AttachTerminalSurfaceRequestSchema,
				clientJson(
					AttachTerminalSurfaceRequestSchema,
					JSON.parse(JSON.stringify(args)),
					true,
				),
			),
		});
		subscription.listeners.set(args.attachmentId, {
			streamId,
			receive: (event) => {
				const item = decodeTerminalEvent(event);
				listener(item);
				initialized = true;
				resolveInitial();
			},
			close: (resynchronize, error) => {
				if (!initialized)
					rejectInitial(
						error ?? new Error("Terminal stream closed before snapshot"),
					);
				if (active && resynchronize) onClosed();
			},
		});
		await Promise.all([attached, initial]);
		return release;
	} catch (error) {
		void release().catch((cleanupError) =>
			console.error("Terminal cleanup failed", cleanupError),
		);
		throw terminalError(error);
	}
}

function terminalError(error: unknown) {
	if (error instanceof ConnectError) {
		const detail = error.findDetails(CommandErrorSchema)[0];
		if (detail)
			return clientJson(
				CommandErrorSchema,
				toJson(CommandErrorSchema, detail),
				false,
			);
	}
	return error;
}

export async function acknowledgeClientStream(
	attachmentId: string,
	sequence: number,
) {
	const client = await getClient();
	await client.ackTerminalSurfaceOutput({
		attachmentId,
		sequence: BigInt(sequence),
	});
}

export async function completeClientRestoration(generation: number) {
	await getClient();
	const active = current;
	if (!active) throw new Error("Daemon connection is unavailable");
	await invoke("complete_desktop_restoration", {
		launchId: active.endpoint.launchId,
		attachmentId: active.attachmentId,
		generation,
	});
}

async function applyClientDesktopSettings(
	client: Client<typeof ClientService>,
) {
	const info = await client.getServerInfo({});
	if (info.desktopSettings)
		await invoke("apply_desktop_settings", { settings: info.desktopSettings });
}

window.addEventListener("pagehide", () => {
	stopped = true;
	connectionAbort.abort();
	session = null;
	current = null;
	listeners.clear();
	refreshListeners.clear();
	connectionListeners.clear();
	activeSubscription = null;
	watchers.clear();
	states.clear();
	stateAbort?.abort();
});

export function firstState<K extends keyof StateValues>(
	target: StateTarget<K>,
): Promise<StateValues[K]> {
	return new Promise((resolve, reject) => {
		const release = subscribeState(
			target,
			(value) => {
				resolve(value);
				queueMicrotask(() => release());
			},
			(error) => {
				reject(error);
				queueMicrotask(() => release());
			},
		);
	});
}
