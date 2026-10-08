import { useStateSubscriptionResult } from "@/hooks/useStateSubscription";
import type { DiffBase, DiffSection } from "@/types/settings";

export function useReviewFileView(
	rootPath: string | null,
	filePath: string | null,
	diffBase: DiffBase,
	section: DiffSection,
) {
	const subscription = useStateSubscriptionResult(
		rootPath && filePath
			? {
					kind: "review-file-view",
					args: [rootPath, filePath, section, diffBase],
				}
			: null,
	);
	const view = subscription.value ?? null;
	const loading = Boolean(
		rootPath &&
			filePath &&
			subscription.value === undefined &&
			!subscription.error,
	);

	const originalContent = view?.kind === "textDiff" ? view.original : "";
	const modifiedContent = view?.kind === "textDiff" ? view.modified : "";
	const hunks = view?.kind === "textDiff" ? view.hunks : null;
	const changeGroups = view?.kind === "textDiff" ? view.changeGroups : null;
	const imageDiff = {
		originalUrl: view?.kind === "image" ? view.originalUrl : null,
		modifiedUrl: view?.kind === "image" ? view.modifiedUrl : null,
		loading,
	};

	return {
		view,
		originalContent,
		modifiedContent,
		hunks,
		changeGroups,
		imageDiff,
		loading,
		error: subscription.error,
	};
}
