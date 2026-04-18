import { invoke } from "@tauri-apps/api/core";
import { useCallback, useEffect, useRef, useState } from "react";
import type { ChangeGroup, Hunk } from "@/lib/computeHunks";

interface DiffHunksResult {
	hunks: Hunk[];
	changeGroups: ChangeGroup[];
}

export function useHunks(
	original: string,
	modified: string,
	filePath?: string,
) {
	const [hunks, setHunks] = useState<Hunk[]>([]);
	const [changeGroups, setChangeGroups] = useState<ChangeGroup[]>([]);
	const [currentIndex, setCurrentIndex] = useState(0);
	const requestIdRef = useRef(0);

	useEffect(() => {
		const requestId = ++requestIdRef.current;

		invoke<DiffHunksResult>("compute_diff_hunks", {
			original,
			modified,
			filePath: filePath ?? null,
		})
			.then((result) => {
				if (requestId !== requestIdRef.current) return;
				setHunks(result.hunks);
				setChangeGroups(result.changeGroups);
				setCurrentIndex(0);
			})
			.catch(() => {
				if (requestId !== requestIdRef.current) return;
				setHunks([]);
				setChangeGroups([]);
				setCurrentIndex(0);
			});
	}, [original, modified, filePath]);

	const safeIndex =
		changeGroups.length === 0
			? -1
			: Math.min(currentIndex, changeGroups.length - 1);

	const currentGroup: ChangeGroup | null =
		safeIndex >= 0 ? changeGroups[safeIndex] : null;

	const goToNext = useCallback(() => {
		setCurrentIndex((prev) =>
			changeGroups.length === 0 ? 0 : (prev + 1) % changeGroups.length,
		);
	}, [changeGroups.length]);

	const goToPrev = useCallback(() => {
		setCurrentIndex((prev) =>
			changeGroups.length === 0
				? 0
				: (prev - 1 + changeGroups.length) % changeGroups.length,
		);
	}, [changeGroups.length]);

	const goTo = useCallback(
		(index: number) => {
			if (index >= 0 && index < changeGroups.length) {
				setCurrentIndex(index);
			}
		},
		[changeGroups.length],
	);

	return {
		hunks,
		changeGroups,
		currentIndex: safeIndex,
		currentGroup,
		total: changeGroups.length,
		goToNext,
		goToPrev,
		goTo,
	};
}
