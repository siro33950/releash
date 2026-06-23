import { invoke } from "@tauri-apps/api/core";
import { useEffect, useRef, useState } from "react";
import type { DiffBase, DiffSection } from "@/types/settings";

interface ReviewTextDiff {
	original: string;
	modified: string;
}

export function useFileDiffContent(
	filePath: string | null,
	diffBase: DiffBase,
	section: DiffSection,
	gitRefreshKey: number,
) {
	const [originalContent, setOriginalContent] = useState("");
	const [modifiedContent, setModifiedContent] = useState("");
	const [loading, setLoading] = useState(false);
	const requestIdRef = useRef(0);

	// biome-ignore lint/correctness/useExhaustiveDependencies: gitRefreshKey is an intentional trigger to re-fetch on git state changes
	useEffect(() => {
		const requestId = ++requestIdRef.current;

		if (!filePath) {
			setOriginalContent("");
			setModifiedContent("");
			setLoading(false);
			return;
		}

		setLoading(true);

		invoke<ReviewTextDiff>("get_review_text_diff", {
			filePath,
			diffBase,
			section,
		})
			.then(({ original, modified }) => {
				if (requestId !== requestIdRef.current) return;
				setOriginalContent(original);
				setModifiedContent(modified);
				setLoading(false);
			})
			.catch(() => {
				if (requestId !== requestIdRef.current) return;
				setOriginalContent("");
				setModifiedContent("");
				setLoading(false);
			});

		return () => {
			requestIdRef.current++;
		};
	}, [filePath, diffBase, section, gitRefreshKey]);

	return { originalContent, modifiedContent, loading };
}
