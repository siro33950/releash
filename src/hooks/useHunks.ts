import { useCallback, useMemo, useState } from "react";
import {
	type ChangeGroup,
	computeChangeGroups,
	computeHunks,
} from "@/lib/computeHunks";

export function useHunks(
	original: string,
	modified: string,
	filePath?: string,
) {
	const hunks = useMemo(
		() => computeHunks(original, modified, filePath),
		[original, modified, filePath],
	);

	const changeGroups = useMemo(
		() => computeChangeGroups(hunks),
		[hunks],
	);

	const [currentIndex, setCurrentIndex] = useState(0);

	const safeIndex =
		changeGroups.length === 0
			? -1
			: Math.min(currentIndex, changeGroups.length - 1);

	const currentGroup: ChangeGroup | null =
		safeIndex >= 0 ? changeGroups[safeIndex] : null;

	const goToNext = useCallback(() => {
		setCurrentIndex((prev) =>
			changeGroups.length === 0
				? 0
				: (prev + 1) % changeGroups.length,
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
