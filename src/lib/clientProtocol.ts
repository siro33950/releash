import type { JsonValue } from "@bufbuild/protobuf";
import {
	create,
	fromBinary,
	fromJson,
	toBinary,
	toJson,
} from "@bufbuild/protobuf";
import {
	type CommandError,
	CommandErrorSchema,
	CommandRequestSchema,
	type CommandResult,
	CommandResultSchema,
	EnvelopeSchema,
	type OperationReference,
	OperationReferenceSchema,
	type Push,
	PushSchema,
	type Stream,
	TerminalEventSchema,
} from "@/generated/client_pb";
import type { ClientPushPayloads } from "@/generated/client_types";
import { clientJson } from "./clientJson";
import type { TerminalSurfaceStreamItem } from "./terminalSurfaceStream";

export const MAX_STREAM_FRAME_BYTES = 64 * 1024;

export function createClientCommand(
	requestId: string,
	command: string,
	args: Record<string, unknown> = {},
	instanceId = "",
	recover = false,
	predecessors: OperationReference[] = [],
	deadlineUnixMs = 0,
	userRetry = false,
	successors: OperationReference[] = [],
) {
	const field = CommandRequestSchema.fields.find(
		(field) => field.name === command,
	);
	if (!field?.message) throw new Error(`Unknown client command: ${command}`);
	const request = fromJson(CommandRequestSchema, {
		requestId,
		instanceId,
		recover,
		deadlineUnixMs: String(deadlineUnixMs),
		userRetry,
		successors: successors.map((reference) =>
			toJson(OperationReferenceSchema, reference),
		),
		predecessors: predecessors.map((reference) =>
			toJson(OperationReferenceSchema, reference),
		),
		[field.jsonName]: clientJson(
			field.message,
			JSON.parse(JSON.stringify(args)),
			true,
		),
	});
	if (!request.command.case)
		throw new Error(`Unknown client command: ${command}`);
	return request;
}

export function encodeClientCommand(
	...args: Parameters<typeof createClientCommand>
) {
	const request = createClientCommand(...args);
	return toBinary(
		EnvelopeSchema,
		create(EnvelopeSchema, { body: { case: "request", value: request } }),
	);
}

export function decodeClientEnvelope(data: ArrayBuffer) {
	return fromBinary(EnvelopeSchema, new Uint8Array(data));
}

export function encodeClientAck(
	attachmentId: string,
	sequence: number,
	outputSequence?: number,
) {
	return toBinary(
		EnvelopeSchema,
		create(EnvelopeSchema, {
			body: {
				case: "ack",
				value: {
					attachmentId,
					sequence: BigInt(sequence),
					outputSequence:
						outputSequence === undefined ? undefined : BigInt(outputSequence),
				},
			},
		}),
	);
}

export function encodeClientRequestAck(
	requestId: string,
	releaseWatch = false,
	confirmWatch = false,
) {
	return toBinary(
		EnvelopeSchema,
		create(EnvelopeSchema, {
			body: {
				case: "requestAck",
				value: { requestId, releaseWatch, confirmWatch },
			},
		}),
	);
}

export function decodeClientValue(
	value: CommandError | CommandResult,
	command?: string,
): JsonValue {
	if (value.$typeName === "releash.client.v1.CommandError")
		return clientJson(
			CommandErrorSchema,
			toJson(CommandErrorSchema, value),
			false,
		);
	const json = toJson(CommandResultSchema, value);
	if (!json || typeof json !== "object" || Array.isArray(json))
		throw new Error("Invalid command result");
	const field = CommandResultSchema.fields.find(
		(field) => field.jsonName in json,
	);
	if (!field?.message) throw new Error("Missing command result");
	if (command && field.name !== command)
		throw new Error("Mismatched command result");
	return clientJson(field.message, json[field.jsonName], false);
}

export function decodeClientPush<K extends keyof ClientPushPayloads>(
	value: Push,
	event: K,
): ClientPushPayloads[K] {
	const field = PushSchema.fields.find(
		(field) => field.name.replace(/_/g, "-") === event,
	);
	const json = toJson(PushSchema, value);
	if (
		!field?.message ||
		!json ||
		typeof json !== "object" ||
		Array.isArray(json) ||
		!(field.jsonName in json)
	)
		throw new Error("Invalid push event");
	return clientJson(
		field.message,
		json[field.jsonName],
		false,
	) as ClientPushPayloads[K];
}

export class ClientStreamDecoder {
	private sequence = 0n;
	private chunks: Uint8Array[] = [];
	private length = 0;

	decode(frame: Stream): TerminalSurfaceStreamItem | null {
		if (frame.sequence !== this.sequence + 1n)
			throw new Error("Out-of-order terminal stream");
		this.sequence = frame.sequence;
		this.chunks.push(frame.data);
		this.length += frame.data.length;
		if (!frame.end) return null;
		const bytes = new Uint8Array(this.length);
		let offset = 0;
		for (const chunk of this.chunks) {
			bytes.set(chunk, offset);
			offset += chunk.length;
		}
		this.chunks = [];
		this.length = 0;
		const { item } = fromBinary(TerminalEventSchema, bytes);
		switch (item.case) {
			case "snapshot":
				return {
					type: "snapshot",
					surface: {
						session_key: item.value.sessionKey,
						terminal_surface: {
							replay: item.value.replay,
							sequence: Number(item.value.sequence),
							cols: item.value.cols,
							rows: item.value.rows,
						},
						is_exited: item.value.isExited,
						exit_code: item.value.exitCode ?? null,
					},
				};
			case "output":
				return {
					type: "output",
					session_key: item.value.sessionKey,
					data: item.value.data,
					sequence: Number(item.value.sequence),
				};
			case "resize":
				return {
					type: "resize",
					session_key: item.value.sessionKey,
					rows: item.value.rows,
					cols: item.value.cols,
					sequence: Number(item.value.sequence),
				};
			case "exit":
				return {
					type: "exit",
					session_key: item.value.sessionKey,
					exit_code: item.value.exitCode ?? null,
					sequence: Number(item.value.sequence),
				};
			case "inputUnavailable":
				return {
					type: "input_unavailable",
					session_key: item.value.sessionKey,
					message: item.value.message,
				};
			default:
				throw new Error("Missing terminal event");
		}
	}
}
