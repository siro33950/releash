import { invoke } from "@tauri-apps/api/core";
import { useEffect, useState } from "react";
import { buildDataUrl, getMimeType } from "@/lib/imageUtils";
import type { DiffBase, DiffSection } from "@/types/settings";

export interface ImageDiffResult {
	originalUrl: string | null;
	modifiedUrl: string | null;
	loading: boolean;
}

interface ReviewImageDiff {
	originalBase64: string | null;
	modifiedBase64: string | null;
}

export function useImageDiff(
	filePath: string | null,
	diffBase: DiffBase,
	section: DiffSection,
	gitRefreshKey?: number,
): ImageDiffResult {
	const [originalUrl, setOriginalUrl] = useState<string | null>(null);
	const [modifiedUrl, setModifiedUrl] = useState<string | null>(null);
	const [loading, setLoading] = useState(false);

	// biome-ignore lint/correctness/useExhaustiveDependencies: gitRefreshKey is an intentional trigger to re-fetch images on git state changes
	useEffect(() => {
		if (!filePath) {
			setOriginalUrl(null);
			setModifiedUrl(null);
			return;
		}

		let cancelled = false;
		setLoading(true);

		const mime = getMimeType(filePath);

		const fetchAll = async () => {
			const { originalBase64, modifiedBase64 } = await invoke<ReviewImageDiff>(
				"get_review_image_diff",
				{
					filePath,
					diffBase,
					section,
				},
			);

			if (!cancelled) {
				setOriginalUrl(
					originalBase64 ? buildDataUrl(originalBase64, mime) : null,
				);
				setModifiedUrl(
					modifiedBase64 ? buildDataUrl(modifiedBase64, mime) : null,
				);
				setLoading(false);
			}
		};

		fetchAll().catch(() => {
			if (!cancelled) {
				setOriginalUrl(null);
				setModifiedUrl(null);
				setLoading(false);
			}
		});

		return () => {
			cancelled = true;
		};
	}, [filePath, diffBase, section, gitRefreshKey]);

	return { originalUrl, modifiedUrl, loading };
}
