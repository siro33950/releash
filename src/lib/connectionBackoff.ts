import { getOption } from "@bufbuild/protobuf";
import { connection_backoff } from "@/generated/client_options_pb";
import { ClientService } from "@/generated/client_pb";

const policy = getOption(ClientService, connection_backoff);

export function createConnectionBackoff() {
	let current: number | undefined;
	return {
		next() {
			if (current === undefined) {
				current = policy.initialBackoffMs;
				return current;
			}
			current = Math.min(current * policy.multiplier, policy.maxBackoffMs);
			const spread = policy.jitter * current;
			return Math.round(current + (Math.random() * 2 - 1) * spread);
		},
		reset() {
			current = undefined;
		},
	};
}
