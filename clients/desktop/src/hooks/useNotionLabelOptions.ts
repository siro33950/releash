import { useStateSubscriptionResult } from "./useStateSubscription";

export function useNotionLabelOptions(repoPath: string) {
	const subscription = useStateSubscriptionResult({
		kind: "notion-label-options",
		args: [repoPath],
	});
	const error =
		subscription.error ?? subscription.value?.readError?.message ?? null;
	return {
		labelOptions: subscription.value?.options ?? [],
		loading: subscription.value === undefined && !error,
		error,
		readError: subscription.value?.readError,
	};
}
