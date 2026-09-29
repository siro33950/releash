import { useStateSubscriptionResult } from "./useStateSubscription";

export function useWorkflowConfig(open: boolean) {
	const subscription = useStateSubscriptionResult(open ? "workflows" : null);
	return {
		workflows: subscription.value ?? [],
		loading: open && subscription.value === undefined && !subscription.error,
		error: subscription.error,
	};
}
