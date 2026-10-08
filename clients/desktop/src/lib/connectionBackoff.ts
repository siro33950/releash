import { getOption } from "@bufbuild/protobuf";
import { connection_backoff } from "@/generated/client_options_pb";
import { ClientService } from "@/generated/client_pb";

const policy = getOption(ClientService, connection_backoff);

export function createConnectionBackoff() {
	let current: number | undefined;
	let lastAttemptAt: number | undefined;
	return {
		next() {
			if (
				lastAttemptAt !== undefined &&
				Date.now() - lastAttemptAt > policy.resetAfterMs
			)
				current = undefined;
			if (current === undefined) {
				current = policy.initialBackoffMs;
				return current;
			}
			current = Math.min(current * policy.multiplier, policy.maxBackoffMs);
			const spread = policy.jitter * current;
			return Math.round(current + (Math.random() * 2 - 1) * spread);
		},
		attemptStarted() {
			lastAttemptAt = Date.now();
		},
	};
}
