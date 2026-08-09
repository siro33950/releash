import { invoke } from "@tauri-apps/api/core";

export interface CachedInvoke<TValue> {
	get(): Promise<TValue>;
	reset(): void;
}

export function createCachedInvoke<TResponse, TValue>(options: {
	command: string;
	normalize: (response: TResponse) => TValue;
	fallback: TValue;
	failureMessage: string;
}): CachedInvoke<TValue> {
	let cached: Promise<TValue> | null = null;
	return {
		get() {
			cached ??= Promise.resolve()
				.then(() => invoke<TResponse>(options.command))
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
