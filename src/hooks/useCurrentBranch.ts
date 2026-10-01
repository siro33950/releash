import { useStateSubscriptionResult } from "./useStateSubscription";

export function useCurrentBranch(rootPath: string | null) {
	const branch = useStateSubscriptionResult(
		rootPath ? { kind: "current-branch", args: [rootPath] } : null,
	);
	return {
		branch: branch.error ? null : (branch.value ?? null),
		error: branch.error,
	};
}
