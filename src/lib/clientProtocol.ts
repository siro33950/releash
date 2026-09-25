import { toJson } from "@bufbuild/protobuf";
import {
	type Push,
	PushSchema,
	type TerminalEvent,
} from "@/generated/client_pb";
import type { ClientPushPayloads } from "@/generated/client_types";
import { clientJson } from "./clientJson";
import type { TerminalSurfaceStreamItem } from "./terminalSurfaceStream";

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

export function decodeTerminalEvent({
	item,
}: TerminalEvent): TerminalSurfaceStreamItem {
	switch (item.case) {
		case "snapshot":
			return {
				type: "snapshot",
				surface: {
					session_key: item.value.sessionKey,
					processed_report_units: item.value.processedReportUnits,
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
		default:
			throw new Error("Missing terminal event");
	}
}
