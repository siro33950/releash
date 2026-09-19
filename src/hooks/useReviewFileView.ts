import { useEffect, useRef, useState } from "react";
import { invokeClient as invoke } from "@/lib/client";
import { getErrorMessage } from "@/lib/errorMessage";
import type { ReviewFileView, ReviewViewport } from "@/types/review";
import type { DiffBase, DiffSection } from "@/types/settings";

export function useReviewFileView(
	rootPath: string | null,
	filePath: string | null,
	diffBase: DiffBase,
	section: DiffSection,
	gitRefreshKey: number,
	snapshotVersion: number | null,
	viewport?: ReviewViewport,
) {
	const [view, setView] = useState<ReviewFileView | null>(null);
	const [loading, setLoading] = useState(false);
	const [error, setError] = useState<string | null>(null);
	const requestIdRef = useRef(0);

	// biome-ignore lint/correctness/useExhaustiveDependencies: gitRefreshKey is an intentional trigger to re-fetch on git state changes
	useEffect(() => {
		const requestId = ++requestIdRef.current;
		if (!rootPath || !filePath) {
			setView(null);
			setLoading(false);
			setError(null);
			return;
		}

		setLoading(true);
		setError(null);
		invoke("get_review_file_view", {
			input: {
				worktreePath: rootPath,
				target: { by: "path", value: filePath },
				section,
				base: diffBase,
				snapshotVersion,
				viewport: viewport ?? null,
			},
		})
			.then(async (result) => {
				if (requestId !== requestIdRef.current) return;
				if (result.kind === "image") {
					const [originalUrl, modifiedUrl] = await Promise.all([
						result.originalUrl
							? invoke("get_review_blob", { reference: result.originalUrl })
							: null,
						result.modifiedUrl
							? invoke("get_review_blob", { reference: result.modifiedUrl })
							: null,
					]);
					result = { ...result, originalUrl, modifiedUrl };
				}
				if (requestId !== requestIdRef.current) return;
				setView(result);
				setLoading(false);
			})
			.catch((reason: unknown) => {
				if (requestId !== requestIdRef.current) return;
				setView(null);
				setError(getErrorMessage(reason));
				setLoading(false);
			});

		return () => {
			requestIdRef.current++;
		};
	}, [
		rootPath,
		filePath,
		diffBase,
		section,
		gitRefreshKey,
		snapshotVersion,
		viewport,
	]);

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
		error,
	};
}
