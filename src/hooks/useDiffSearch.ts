import { useCallback, useMemo, useState } from "react";
import type { DiffLine } from "@/hooks/useDiffTokens";

export interface SearchMatch {
	lineIndex: number;
	startOffset: number;
	endOffset: number;
}

export interface DiffSearchState {
	query: string;
	isOpen: boolean;
	matches: SearchMatch[];
	currentIndex: number;
	totalMatches: number;
	setQuery: (query: string) => void;
	open: () => void;
	close: () => void;
	goToNext: () => void;
	goToPrev: () => void;
}

function findMatches(lines: DiffLine[], query: string): SearchMatch[] {
	if (query === "") return [];

	const lowerQuery = query.toLowerCase();
	const results: SearchMatch[] = [];

	for (let i = 0; i < lines.length; i++) {
		const line = lines[i];
		const lowerContent = line.content.toLowerCase();
		let searchFrom = 0;

		while (searchFrom < lowerContent.length) {
			const idx = lowerContent.indexOf(lowerQuery, searchFrom);
			if (idx === -1) break;
			results.push({
				lineIndex: i,
				startOffset: idx,
				endOffset: idx + query.length,
			});
			searchFrom = idx + 1;
		}
	}

	return results;
}

export function useDiffSearch(lines: DiffLine[]): DiffSearchState {
	const [query, setQuery] = useState("");
	const [isOpen, setIsOpen] = useState(false);
	const [currentIndex, setCurrentIndex] = useState(0);

	const matches = useMemo(() => findMatches(lines, query), [lines, query]);

	const safeCurrentIndex =
		matches.length === 0
			? -1
			: currentIndex >= matches.length
				? 0
				: currentIndex;

	const open = useCallback(() => {
		setIsOpen(true);
	}, []);

	const close = useCallback(() => {
		setIsOpen(false);
		setQuery("");
		setCurrentIndex(0);
	}, []);

	const handleSetQuery = useCallback((q: string) => {
		setQuery(q);
		setCurrentIndex(0);
	}, []);

	const goToNext = useCallback(() => {
		if (matches.length === 0) return;
		setCurrentIndex((prev) => (prev + 1) % matches.length);
	}, [matches.length]);

	const goToPrev = useCallback(() => {
		if (matches.length === 0) return;
		setCurrentIndex((prev) => (prev - 1 + matches.length) % matches.length);
	}, [matches.length]);

	return {
		query,
		isOpen,
		matches,
		currentIndex: safeCurrentIndex,
		totalMatches: matches.length,
		setQuery: handleSetQuery,
		open,
		close,
		goToNext,
		goToPrev,
	};
}
