import type { ClientCommandResults } from "@/generated/client_types";
import {
	type EmptyClientCommand,
	invokeClient as invoke,
} from "@/lib/clientSocket";

export interface CachedInvoke<TValue> {
	get(): Promise<TValue>;
	reset(): void;
}

export function createCachedInvoke<
	K extends EmptyClientCommand,
	TValue,
>(options: {
	command: K;
	normalize: (response: ClientCommandResults[K]) => TValue;
	fallback: TValue;
	failureMessage: string;
}): CachedInvoke<TValue> {
	let cached: Promise<TValue> | null = null;
	return {
		get() {
			cached ??= Promise.resolve()
				.then(() => invoke(options.command))
				.then(options.normalize)
				.catch((error) => {
					console.warn(options.failureMessage, error);
					cached = null;
					return options.fallback;
				});
			return cached;
		},
		reset() {
			cached = null;
		},
	};
}
