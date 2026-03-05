import { useCallback, useEffect, useMemo, useState } from "react";
import type { Thread } from "@/types/thread";

export interface UseUnresolvedNavigationResult {
	currentIndex: number;
	total: number;
	currentThread: Thread | null;
	goNext: () => void;
	goPrev: () => void;
	goToThread: (threadId: string) => void;
}

export function useUnresolvedNavigation(
	threads: Thread[],
	onNavigate?: (filePath: string, lineNumber: number) => void,
): UseUnresolvedNavigationResult {
	const [currentIndex, setCurrentIndex] = useState(-1);

	const sorted = useMemo(() => {
		return threads
			.filter((t) => !t.resolved)
			.sort((a, b) => {
				const fileCmp = a.filePath.localeCompare(b.filePath);
				if (fileCmp !== 0) return fileCmp;
				return a.lineNumber - b.lineNumber;
			});
	}, [threads]);

	const total = sorted.length;

	useEffect(() => {
		if (total === 0) {
			setCurrentIndex(-1);
		} else if (currentIndex >= total) {
			setCurrentIndex(total - 1);
		}
	}, [total, currentIndex]);

	const currentThread = useMemo(() => {
		if (currentIndex >= 0 && currentIndex < total) {
			return sorted[currentIndex];
		}
		return null;
	}, [sorted, currentIndex, total]);

	const navigateTo = useCallback(
		(index: number) => {
			if (total === 0) return;
			const clamped = ((index % total) + total) % total;
			setCurrentIndex(clamped);
			const thread = sorted[clamped];
			if (thread && onNavigate) {
				onNavigate(thread.filePath, thread.lineNumber);
			}
		},
		[sorted, total, onNavigate],
	);

	const goNext = useCallback(() => {
		navigateTo(currentIndex + 1);
	}, [currentIndex, navigateTo]);

	const goPrev = useCallback(() => {
		navigateTo(currentIndex <= 0 ? total - 1 : currentIndex - 1);
	}, [currentIndex, total, navigateTo]);

	const goToThread = useCallback(
		(threadId: string) => {
			const idx = sorted.findIndex((t) => t.id === threadId);
			if (idx >= 0) {
				navigateTo(idx);
			}
		},
		[sorted, navigateTo],
	);

	return {
		currentIndex,
		total,
		currentThread,
		goNext,
		goPrev,
		goToThread,
	};
}
