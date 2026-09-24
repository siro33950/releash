import { useEffect, useState } from "react";
import {
	type StateTarget,
	type StateValues,
	subscribeState,
} from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";

export function useStateSubscriptionResult<K extends keyof StateValues>(
	target: StateTarget<K> | null,
) {
	const key = JSON.stringify(target);
	const [state, setState] = useState<{
		key: string;
		value?: StateValues[K];
		error: string | null;
	}>();
	useEffect(() => {
		const current: StateTarget<K> | null = JSON.parse(key);
		if (!current) return;
		return subscribeState(
			current,
			(value) => setState({ key, value, error: null }),
			(error) =>
				setState((previous) => ({
					key,
					value: previous?.key === key ? previous.value : undefined,
					error: getErrorMessage(error),
				})),
		);
	}, [key]);
	return state?.key === key ? state : { value: undefined, error: null };
}

export function useStateSubscription<K extends keyof StateValues>(
	target: StateTarget<K> | null,
) {
	return useStateSubscriptionResult(target).value;
}
