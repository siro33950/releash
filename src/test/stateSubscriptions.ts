import { vi } from "vitest";
import type { StateTarget, StateValues } from "@/lib/client";

export function stateSubscriptions() {
	const values = new Map<string, unknown>();
	const failures = new Map<string, unknown>();
	const listeners = new Map<string, Set<(value: never) => void>>();
	const errors = new Map<string, Set<(error: unknown) => void>>();
	const key = (target: StateTarget<keyof StateValues>) =>
		JSON.stringify(target);
	const subscribeState = vi.fn(
		<K extends keyof StateValues>(
			target: StateTarget<K>,
			receive: (value: StateValues[K]) => void,
			onError: (error: unknown) => void,
		) => {
			const id = key(target);
			const handlers = errors.get(id) ?? new Set();
			errors.set(id, handlers);
			handlers.add(onError);
			const receivers = listeners.get(id) ?? new Set();
			listeners.set(id, receivers);
			receivers.add(receive as (value: never) => void);
			if (values.has(id)) receive(values.get(id) as StateValues[K]);
			if (failures.has(id)) onError(failures.get(id));
			return () => {
				receivers.delete(receive as (value: never) => void);
				handlers.delete(onError);
				if (!receivers.size) failures.delete(id);
			};
		},
	);
	return {
		subscribeState,
		firstState: vi.fn(
			<K extends keyof StateValues>(
				target: StateTarget<K>,
			): Promise<StateValues[K]> =>
				values.has(key(target))
					? Promise.resolve(values.get(key(target)) as StateValues[K])
					: Promise.reject(new Error("No fixture for state target")),
		),
		publish<K extends keyof StateValues>(
			target: StateTarget<K>,
			value: StateValues[K],
		) {
			const id = key(target);
			failures.delete(id);
			values.set(id, value);
			for (const receive of listeners.get(id) ?? []) receive(value as never);
		},
		fail(target: StateTarget<keyof StateValues>, error: unknown) {
			const id = key(target);
			failures.set(id, error);
			for (const handler of errors.get(id) ?? []) handler(error);
		},
		clear() {
			values.clear();
			failures.clear();
			listeners.clear();
			errors.clear();
			subscribeState.mockClear();
		},
	};
}
