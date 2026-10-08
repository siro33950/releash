import { ChevronDown, ChevronUp, X } from "lucide-react";
import { useCallback, useEffect, useRef } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

interface DiffSearchBarProps {
	query: string;
	onQueryChange: (query: string) => void;
	currentIndex: number;
	totalMatches: number;
	onNext: () => void;
	onPrev: () => void;
	onClose: () => void;
}

export function DiffSearchBar({
	query,
	onQueryChange,
	currentIndex,
	totalMatches,
	onNext,
	onPrev,
	onClose,
}: DiffSearchBarProps) {
	const inputRef = useRef<HTMLInputElement>(null);

	useEffect(() => {
		inputRef.current?.focus();
	}, []);

	const handleKeyDown = useCallback(
		(e: React.KeyboardEvent) => {
			if (e.key === "Escape") {
				e.preventDefault();
				onClose();
			} else if (e.key === "Enter" && e.shiftKey) {
				e.preventDefault();
				onPrev();
			} else if (e.key === "Enter") {
				e.preventDefault();
				onNext();
			}
		},
		[onClose, onNext, onPrev],
	);

	const matchLabel =
		totalMatches === 0
			? query.length > 0
				? "0"
				: ""
			: `${currentIndex + 1}/${totalMatches}`;

	return (
		<div
			className="absolute top-0 right-4 z-10 flex items-center gap-1 rounded-b-md border border-t-0 border-border bg-[var(--editor-background,#1a1a1a)] px-2 py-1 shadow-md"
			data-testid="diff-search-bar"
		>
			<Input
				ref={inputRef}
				variant="panel"
				size="xs"
				className="w-40"
				placeholder="Find..."
				value={query}
				onChange={(e) => onQueryChange(e.target.value)}
				onKeyDown={handleKeyDown}
				data-testid="diff-search-input"
			/>
			{matchLabel && (
				<span
					className="text-xs text-muted-foreground whitespace-nowrap min-w-[32px] text-center"
					data-testid="diff-search-count"
				>
					{matchLabel}
				</span>
			)}
			<Button
				variant="ghost"
				size="icon-xs"
				onClick={onPrev}
				disabled={totalMatches === 0}
				aria-label="Previous match"
				data-testid="diff-search-prev"
			>
				<ChevronUp />
			</Button>
			<Button
				variant="ghost"
				size="icon-xs"
				onClick={onNext}
				disabled={totalMatches === 0}
				aria-label="Next match"
				data-testid="diff-search-next"
			>
				<ChevronDown />
			</Button>
			<Button
				variant="ghost"
				size="icon-xs"
				onClick={onClose}
				aria-label="Close search"
				data-testid="diff-search-close"
			>
				<X />
			</Button>
		</div>
	);
}
