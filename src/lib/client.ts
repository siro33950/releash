import { getOption, toJson } from "@bufbuild/protobuf";
import { type Client, ConnectError, createClient } from "@connectrpc/connect";
import { createLinkedAbortController } from "@connectrpc/connect/protocol";
import { createConnectTransport } from "@connectrpc/connect-web";
import { invoke } from "@tauri-apps/api/core";
import {
	default_timeout_ms,
	min_connect_timeout_ms,
	reconnect_status_code,
	state_stream_silence_ms,
} from "@/generated/client_options_pb";
import {
	ClientService,
	type StatePayload,
	StatePayloadSchema,
	type StateVersion,
} from "@/generated/client_pb";
import { clientJson } from "./clientJson";
import { decodeTerminalEvent } from "./clientProtocol";
import { createConnectionBackoff } from "./connectionBackoff";
import type { TerminalSurfaceStreamItem } from "./terminalSurfaceStream";

export {
	type ClientCommand,
	type EmptyClientCommand,
	invokeClient,
} from "@/generated/client_commands";

type Endpoint = { url: string; token: string; launchId: string };
type Session = {
	client: Client<typeof ClientService>;
};
let connectionAbort = new AbortController();
type ConnectionState =
	| { phase: "IDLE" | "TRANSIENT_FAILURE" | "SHUTDOWN" }
	| { phase: "CONNECTING"; pending: Promise<Session> }
	| { phase: "READY"; session: Session };
let connectionState: ConnectionState = { phase: "IDLE" };
const stateListeners = new Set<() => void>();
let connectionBackoff = createConnectionBackoff();
let retryTimer: ReturnType<typeof setTimeout> | undefined;

export function getConnectionState(): ConnectionState["phase"] {
	return connectionState.phase;
}

export function onConnectionStateChange(listener: () => void) {
	stateListeners.add(listener);
	return () => stateListeners.delete(listener);
}

function setConnectionState(next: ConnectionState) {
	connectionState = next;
	for (const listener of stateListeners) listener();
}

function waitForConnectionChange(phase: ConnectionState["phase"]) {
	if (connectionState.phase !== phase) return Promise.resolve();
	return new Promise<void>((resolve) => {
		const release = onConnectionStateChange(() => {
			if (connectionState.phase === phase) return;
			release();
			resolve();
		});
	});
}

async function open(abort: AbortController): Promise<Session> {
	const timeout = setTimeout(
		() => abort.abort(new Error("Daemon connection timed out")),
		getOption(ClientService, min_connect_timeout_ms),
	);
	try {
		const expired = new Promise<never>((_, reject) =>
			abort.signal.addEventListener(
				"abort",
				() => reject(abort.signal.reason),
				{
					once: true,
				},
			),
		);
		const endpoint = await Promise.race([
			invoke<Endpoint>("get_client_endpoint"),
			expired,
		]);
		abort.signal.throwIfAborted();
		const client = createClient(
			ClientService,
			createConnectTransport({
				baseUrl: endpoint.url,
				useBinaryFormat: true,
				defaultTimeoutMs: getOption(ClientService, default_timeout_ms),
				interceptors: [
					(next) => async (request) => {
						request.header.set("Authorization", `Bearer ${endpoint.token}`);
						return next(request);
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
		const info = await Promise.race([
			client.getServerInfo({}, { signal: abort.signal }),
			expired,
		]);
		await Promise.race([
			invoke("validate_daemon_connection", {
				launchId: info.launchId,
				release: info.release,
			}),
			expired,
		]);
		abort.signal.throwIfAborted();
		return { client };
	} finally {
		clearTimeout(timeout);
	}
}

function connect() {
	if (connectionState.phase === "SHUTDOWN") return;
	clearTimeout(retryTimer);
	connectionAbort = new AbortController();
	connectionBackoff.attemptStarted();
	const pending = open(connectionAbort);
	setConnectionState({ phase: "CONNECTING", pending });
	void pending.then(
		(ready) => {
			if (
				connectionState.phase !== "CONNECTING" ||
				connectionState.pending !== pending
			)
				return;
			setConnectionState({ phase: "READY", session: ready });
		},
		() => {
			if (
				connectionState.phase === "CONNECTING" &&
				connectionState.pending === pending
			)
				failConnection();
		},
	);
}

function failConnection(failedClient?: Client<typeof ClientService>) {
	if (
		failedClient &&
		(connectionState.phase !== "READY" ||
			connectionState.session.client !== failedClient)
	)
		return;
	if (
		connectionState.phase === "TRANSIENT_FAILURE" ||
		connectionState.phase === "SHUTDOWN"
	)
		return;
	connectionAbort.abort();
	setConnectionState({ phase: "TRANSIENT_FAILURE" });
	retryTimer = setTimeout(connect, connectionBackoff.next());
}

export async function getClient(): Promise<Client<typeof ClientService>> {
	if (connectionState.phase === "IDLE") connect();
	if (connectionState.phase === "READY") return connectionState.session.client;
	if (connectionState.phase !== "CONNECTING")
		throw new Error(`Daemon connection is ${connectionState.phase}`);
	return (await connectionState.pending).client;
}

export type StateValues = {
	terminal: TerminalSurfaceStreamItem;
	"repository-paths": string[];
	workspaces: import("@/generated/client_types").WorkspaceListSnapshot;
	selection: import("@/generated/client_types").WorkspaceTreeSelectionSnapshot;
	"node-detail":
		| import("@/generated/client_types").WorkspaceNodeDetailDto
		| null;
	"agent-session":
		| import("@/generated/client_types").AgentSessionItemDto
		| null;
	"session-history": import("@/generated/client_types").AgentSessionHistoryPageDto;
	providers: import("@/generated/client_types").AgentSessionProviderDto[];
	branches: import("@/generated/client_types").BranchDto[];
	"branch-base": string | null;
	"branch-status": import("@/generated/client_types").ListBranchStatus;
	"current-branch": string;
	issues: import("@/generated/client_types").IssuesSnapshot;
	"notion-tasks": import("@/generated/client_types").NotionTasksSnapshot;
	"notion-label-options": import("@/generated/client_types").NotionLabelOptionsSnapshot;
	worktrees: import("@/generated/client_types").WorktreeEntryDto[];
	"startup-repository":
		| import("@/generated/client_types").StartupWorktree
		| null;
	"workspace-state":
		| import("@/generated/client_types").WorkspaceStateDto
		| null;
	"review-snapshot": import("@/types/review").ReviewSnapshot;
	"review-file-view": import("@/types/review").ReviewFileView;
	"review-threads": import("@/types/diffComment").ReviewDiscussionThread[];
	workflows: import("@/generated/client_types").WorkflowSummaryDto[];
	workflow: import("@/generated/client_types").WorkflowDto | null;
	"workflow-source": string | null;
	facets: import("@/generated/client_types").FacetSummaryDto[];
	facet: string;
	diagnostics: import("@/generated/client_types").DiagnosticReport;
	"desktop-settings": import("@/generated/client_types").DesktopSettings;
	"notion-config":
		| import("@/generated/client_types").NotionRepoConfigView
		| null;
	"provider-availability": import("@/generated/client_types").ProviderAvailabilitySnapshotResponse;
	"external-editor": import("@/generated/client_types").ExternalEditorState;
	"releash-base": string | null;
	"workflow-config": import("@/generated/client_types").WorkflowSection;
	"provider-hook-health": import("@/generated/client_types").ProviderHookHealthSnapshot;
};
export type StateTarget<K extends keyof StateValues> =
	| K
	| { kind: K; args: string[] };
function stateTargetKey(kind: string, args: string[]) {
	return JSON.stringify([kind, args]);
}
type StateEntry = {
	subscriptionId?: string;
	started?: boolean;
	kind: string;
	args: string[];
	receivers: Set<(value: never) => void>;
	errors: Set<(error: unknown) => void>;
	error?: { current: unknown };
	version?: StateVersion;
	value?: { current: unknown };
};
type StateStream = { client: Client<typeof ClientService>; id: string };
const STATE_SILENCE_MS = getOption(ClientService, state_stream_silence_ms);
const IDLE = "idle";
const RETRY = "retry";
const RECONNECT_CODES = new Set<number>(
	getOption(ClientService, reconnect_status_code),
);
function reconnects(error: unknown) {
	return error instanceof ConnectError && RECONNECT_CODES.has(error.code);
}
const states = new Map<string, StateEntry>();
const subscriptions = new Map<string, StateEntry>();
let stateStream: StateStream | null = null;
let stateAbort: AbortController | null = null;
let stateTask: Promise<void> | null = null;

function decodeState(payload: StatePayload | undefined) {
	if (payload?.value.case === "terminal")
		return { current: decodeTerminalEvent(payload.value.value) };
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

const stateOperations = new Map<string, Promise<void>>();
const terminalStarts = new Map<string, Promise<void>>();
function queueStateOperation(
	target: string,
	operation: () => Promise<unknown>,
) {
	const previous = stateOperations.get(target) ?? Promise.resolve();
	const next = previous.then(operation).then(
		() => {},
		() => {},
	);
	stateOperations.set(target, next);
	void next.then(() => {
		if (stateOperations.get(target) === next) stateOperations.delete(target);
	});
	return next;
}

function startState(stream: StateStream, target: string) {
	const entry = states.get(target);
	if (!entry) return;
	if (entry.subscriptionId) {
		subscriptions.delete(entry.subscriptionId);
		stopState(stream, entry.subscriptionId);
	}
	const subscriptionId = crypto.randomUUID();
	entry.subscriptionId = subscriptionId;
	entry.started = false;
	subscriptions.set(subscriptionId, entry);
	const previousStart =
		entry.kind === "terminal" ? terminalStarts.get(target) : undefined;
	const started = queueStateOperation(subscriptionId, async () => {
		await previousStart;
		return stream.client
			.startStateSubscription({
				clientId: stream.id,
				target: entry.kind,
				args: entry.args,
				version: entry.version,
				subscriptionId,
			})
			.then(() => {
				if (
					entry.subscriptionId === subscriptionId &&
					states.get(target) === entry &&
					stateStream === stream
				)
					entry.started = true;
			})
			.catch((error) => {
				if (
					states.get(target) !== entry ||
					stateStream !== stream ||
					entry.subscriptionId !== subscriptionId
				)
					return;
				if (reconnects(error)) {
					stateAbort?.abort(RETRY);
					return;
				}
				console.error("State subscription failed", error);
				entry.error = { current: error };
				for (const receiver of entry.errors) receiver(error);
			});
	});
	if (entry.kind === "terminal") {
		terminalStarts.set(target, started);
		void started.then(() => {
			if (terminalStarts.get(target) === started) terminalStarts.delete(target);
		});
	}
}

function stopState(stream: StateStream, subscriptionId: string) {
	queueStateOperation(subscriptionId, () =>
		stream.client.stopStateSubscription({ subscriptionId }).catch((error) => {
			if (reconnects(error) && stateStream === stream) stateAbort?.abort(RETRY);
			else console.debug("State unsubscribe failed", error);
		}),
	);
}

function ensureStateStream() {
	if (stateTask || connectionState.phase === "SHUTDOWN" || !states.size) return;
	stateTask = (async () => {
		while (connectionState.phase !== "SHUTDOWN" && states.size) {
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
				connectionBackoff.attemptStarted();
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
					const entry = subscriptions.get(event.subscriptionId);
					if (!entry) continue;
					entry.version = event.version;
					if (event.event.case === "failure") {
						const error = new ConnectError(
							event.event.value.message,
							event.event.value.code,
						);
						entry.error = { current: error };
						for (const receiver of entry.errors) receiver(error);
						continue;
					}
					const value = decodeState(
						event.event.case === "snapshot"
							? event.event.value
							: event.event.case === "change"
								? event.event.value.payload
								: undefined,
					);
					if (!value) continue;
					entry.error = undefined;
					const delta =
						event.event.case === "change" && event.event.value.delta;
					if (!delta && entry.kind !== "terminal") entry.value = value;
					for (const receiver of entry.receivers)
						receiver(value.current as never);
				}
			} catch (error) {
				if (getConnectionState() === "SHUTDOWN") break;
				if (abort.signal.reason !== IDLE && abort.signal.reason !== RETRY)
					console.debug("State stream ended", error);
			} finally {
				clearTimeout(silence);
				subscriptions.clear();
				for (const entry of states.values()) {
					entry.subscriptionId = undefined;
					entry.started = false;
				}
				stateStream = null;
				if (stateAbort === abort) stateAbort = null;
			}
			if (abort.signal.reason === IDLE) continue;
			if (abort.signal.reason === RETRY) {
				await new Promise((resolve) =>
					setTimeout(resolve, connectionBackoff.next()),
				);
				continue;
			}
			if (getConnectionState() !== "SHUTDOWN" && states.size) {
				if (client) failConnection(client);
				if (connectionState.phase === "TRANSIENT_FAILURE")
					await waitForConnectionChange("TRANSIENT_FAILURE");
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
	onError: (error: unknown) => void,
) {
	const kind = typeof input === "string" ? input : input.kind;
	const args = typeof input === "string" ? [] : input.args;
	const target = stateTargetKey(kind, args);
	const receiver = onValue as (value: never) => void;
	let entry = states.get(target);
	if (!entry) {
		entry = {
			kind,
			args,
			receivers: new Set(),
			errors: new Set(),
		};
		states.set(target, entry);
		if (stateStream) startState(stateStream, target);
	} else if (kind === "terminal") {
		entry.error = undefined;
		entry.version = undefined;
		if (stateStream) startState(stateStream, target);
	} else {
		if (entry.value) onValue(entry.value.current as StateValues[K]);
		if (entry.error) onError(entry.error.current);
	}
	entry.receivers.add(receiver);
	entry.errors.add(onError);
	if (connectionState.phase !== "SHUTDOWN") ensureStateStream();
	return () => {
		const current = states.get(target);
		if (current !== entry) return;
		current.receivers.delete(receiver);
		current.errors.delete(onError);
		if (current.receivers.size) return;
		states.delete(target);
		if (current.subscriptionId) {
			subscriptions.delete(current.subscriptionId);
			if (stateStream) stopState(stateStream, current.subscriptionId);
		}
		if (!states.size) {
			stateStream = null;
			stateAbort?.abort(IDLE);
		}
	};
}

export async function subscribeTerminalState(
	args: {
		owner: import("./terminalSurfaceStream").TerminalSurfaceOwner;
	},
	listener: (item: TerminalSurfaceStreamItem) => void,
	onClosed: () => void,
): Promise<() => Promise<void>> {
	const targetArgs = terminalTargetArgs(args.owner);
	let release = () => {};
	let initialized = false;
	await new Promise<void>((resolve, reject) => {
		release = subscribeState(
			{ kind: "terminal", args: targetArgs },
			(item) => {
				listener(item);
				initialized = true;
				resolve();
			},
			(error) => {
				reject(error);
				if (initialized) onClosed();
			},
		);
	}).catch((error) => {
		release();
		throw error;
	});
	const id = states.get(stateTargetKey("terminal", targetArgs))?.subscriptionId;
	if (id) await stateOperations.get(id);
	return async () => release();
}

export function currentTerminalInputId(
	owner: import("./terminalSurfaceStream").TerminalSurfaceOwner,
) {
	const args = terminalTargetArgs(owner);
	const entry = states.get(stateTargetKey("terminal", args));
	return entry?.started ? (entry.subscriptionId ?? null) : null;
}

export async function reportTerminalProcessed(
	owner: import("./terminalSurfaceStream").TerminalSurfaceOwner,
	units: number,
) {
	const stream = stateStream;
	const subscriptionId = states.get(
		stateTargetKey("terminal", terminalTargetArgs(owner)),
	)?.subscriptionId;
	if (!stream || !subscriptionId) return;
	await stream.client.reportTerminalProcessed({
		subscriptionId,
		units,
	});
}

function terminalTargetArgs(
	owner: import("./terminalSurfaceStream").TerminalSurfaceOwner,
) {
	return owner.kind === "session"
		? [owner.workspacePath, owner.sessionId]
		: [owner.workspacePath];
}

window.addEventListener("pagehide", (event) => {
	if (connectionState.phase === "SHUTDOWN") return;
	clearTimeout(retryTimer);
	connectionAbort.abort();
	stateAbort?.abort();
	if (event.persisted) {
		if (
			connectionState.phase === "READY" ||
			connectionState.phase === "CONNECTING"
		)
			setConnectionState({ phase: "TRANSIENT_FAILURE" });
	} else {
		setConnectionState({ phase: "SHUTDOWN" });
		states.clear();
	}
});

window.addEventListener("pageshow", (event) => {
	if (!event.persisted || connectionState.phase === "SHUTDOWN") return;
	connectionAbort = new AbortController();
	connectionBackoff = createConnectionBackoff();
	if (connectionState.phase === "TRANSIENT_FAILURE") connect();
	ensureStateStream();
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
