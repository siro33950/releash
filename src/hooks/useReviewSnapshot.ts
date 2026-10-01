import { useStateSubscriptionResult } from "@/hooks/useStateSubscription";
import type { ReviewSnapshot } from "@/types/review";
import type { DiffBase } from "@/types/settings";

const EMPTY_SNAPSHOT: ReviewSnapshot = {
	version: 0,
	stale: false,
	loading: false,
	base: "head",
	files: [],
	stagedFiles: [],
	changedFiles: [],
	diffStats: [],
	tree: [],
	stagedTree: [],
	changesTree: [],
	stagedFileCount: 0,
	changesFileCount: 0,
};

export function useReviewSnapshot(rootPath: string | null, diffBase: DiffBase) {
	const subscription = useStateSubscriptionResult(
		rootPath ? { kind: "review-snapshot", args: [rootPath, diffBase] } : null,
	);
	const snapshot = subscription.value ?? {
		...EMPTY_SNAPSHOT,
		base: diffBase,
	};

	return {
		error: subscription.error,
		snapshot,
		files: snapshot.files,
		stagedFiles: snapshot.stagedFiles,
		changedFiles: snapshot.changedFiles,
		stagedTree: snapshot.stagedTree,
		changesTree: snapshot.changesTree,
		branchBaseTree: snapshot.tree,
		stagedFileCount: snapshot.stagedFileCount,
		changesFileCount: snapshot.changesFileCount,
		branchBaseFileCount: snapshot.files.length,
		version: snapshot.version,
		loading: Boolean(
			rootPath && subscription.value === undefined && !subscription.error,
		),
	};
}
