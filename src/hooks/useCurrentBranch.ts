import { useStateSubscription } from "./useStateSubscription";

export function useCurrentBranch(rootPath: string | null) {
	const branch = useStateSubscription(
		rootPath ? { kind: "current-branch", args: [rootPath] } : null,
	);
	return { branch: branch ?? null };
}
