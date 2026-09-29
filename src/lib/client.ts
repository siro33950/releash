import { getOption, toJson } from "@bufbuild/protobuf";
import { type Client, ConnectError, createClient } from "@connectrpc/connect";
import { createLinkedAbortController } from "@connectrpc/connect/protocol";
import { createConnectTransport } from "@connectrpc/connect-web";
import { invoke } from "@tauri-apps/api/core";
import {
	default_timeout_ms,
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
	endpoint: Endpoint;
	attachmentId: string;
};
let session: Promise<Session> | null = null;
let current: Session | null = null;
let connectionAbort = new AbortController();
let stopped = false;
const connectionListeners = new Set<(connected: boolean) => void>();

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
	const info = await client.getServerInfo({});
	await invoke("validate_daemon_connection", {
		launchId: info.launchId,
		release: info.release,
	});
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
	current = null;
	session = null;
	connectionAbort.abort();
	connectionAbort = new AbortController();
	if (previous) for (const listener of connectionListeners) listener(false);
}

export type StateValues = {
	failures: import("@/generated/client_types").FailureRecords;
	terminal: TerminalSurfaceStreamItem;
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
	"performance-switches": import("@/generated/client_types").PerformanceSwitchesV1;
	"provider-hook-health": import("@/generated/client_types").ProviderHookHealthWarningResponse[];
	"startup-outcome": import("@/generated/client_types").ApplicationStartupOutcomeDtoV1;
};
export type StateTarget<K extends keyof StateValues> =
	| K
	| { kind: K; args: string[] };
function stateTargetKey(kind: string, args: string[]) {
	return JSON.stringify([kind, args]);
}
type StateEntry = {
	terminalInputId?: string;
	kind: string;
	args: string[];
	receivers: Set<(value: never) => void>;
	errors: Set<(error: unknown) => void>;
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

// 同じ対象の開始と停止は送った順に daemon へ届ける。並行に送ると順序が入れ替わり、
// 開始済みの対象への開始は無視されるため、後から届いた停止で購読が消える。
const stateOperations = new Map<string, Promise<void>>();
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
}

function startState(stream: StateStream, target: string) {
	const entry = states.get(target);
	if (!entry) return;
	queueStateOperation(target, () =>
		stream.client
			.startStateSubscription({
				clientId: stream.id,
				target: entry.kind,
				args: entry.args,
				version: entry.version,
				terminalInputId: entry.terminalInputId,
			})
			.catch((error) => {
				if (states.get(target) !== entry || stateStream !== stream) return;
				if (reconnects(error)) {
					stateAbort?.abort(RETRY);
					return;
				}
				console.error("State subscription failed", error);
				for (const receiver of entry.errors) receiver(error);
			}),
	);
}

function ensureStateStream() {
	if (stateTask || stopped || !states.size) return;
	stateTask = (async () => {
		const backoff = createConnectionBackoff();
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
					const delta =
						event.event.case === "change" && event.event.value.delta;
					if (!delta && entry.kind !== "terminal") entry.value = value;
					for (const receiver of entry.receivers)
						receiver(value.current as never);
				}
			} catch (error) {
				if (stopped) break;
				if (abort.signal.reason !== IDLE && abort.signal.reason !== RETRY)
					console.debug("State stream ended", error);
			} finally {
				clearTimeout(silence);
				stateStream = null;
				if (stateAbort === abort) stateAbort = null;
			}
			if (abort.signal.reason === IDLE) continue;
			if (!stopped && states.size) {
				if (abort.signal.reason !== RETRY) refreshClient(client);
				const delay = new AbortController();
				stateAbort = delay;
				await new Promise<void>((resolve) => {
					const timer = setTimeout(resolve, backoff.next());
					delay.signal.addEventListener(
						"abort",
						() => {
							clearTimeout(timer);
							resolve();
						},
						{ once: true },
					);
				});
				if (stateAbort === delay) stateAbort = null;
				if (!stopped && states.size) backoff.attemptStarted();
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
	terminalInputId?: string,
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
			terminalInputId,
			receivers: new Set(),
			errors: new Set(),
		};
		states.set(target, entry);
		if (stateStream) startState(stateStream, target);
	} else if (kind === "terminal") {
		if (terminalInputId) entry.terminalInputId = terminalInputId;
		entry.version = undefined;
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
		} else if (stateStream) {
			const stream = stateStream;
			queueStateOperation(target, () =>
				stream.client
					.stopStateSubscription({ clientId: stream.id, target: kind, args })
					.catch((error) => {
						if (reconnects(error) && stateStream === stream)
							stateAbort?.abort(RETRY);
						else console.debug("State unsubscribe failed", error);
					}),
			);
		}
	};
}

export function onClientConnection(listener: (connected: boolean) => void) {
	connectionListeners.add(listener);
	return () => {
		connectionListeners.delete(listener);
	};
}

export async function subscribeTerminalState(
	args: {
		owner: import("./terminalSurfaceStream").TerminalSurfaceOwner;
		attachmentId: string;
	},
	listener: (item: TerminalSurfaceStreamItem) => void,
	onClosed: () => void,
): Promise<() => Promise<void>> {
	const owner = args.owner;
	const targetArgs =
		owner.kind === "session"
			? [owner.workspacePath, owner.sessionId]
			: [owner.workspacePath];
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
			args.attachmentId,
		);
	}).catch((error) => {
		release();
		throw error;
	});
	return async () => release();
}

export async function reportTerminalProcessed(
	owner: import("./terminalSurfaceStream").TerminalSurfaceOwner,
	units: number,
) {
	const stream = stateStream;
	if (!stream) return;
	await stream.client.reportTerminalProcessed({
		clientId: stream.id,
		args:
			owner.kind === "session"
				? [owner.workspacePath, owner.sessionId]
				: [owner.workspacePath],
		units,
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

window.addEventListener("pagehide", () => {
	stopped = true;
	connectionAbort.abort();
	session = null;
	current = null;
	connectionListeners.clear();
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
