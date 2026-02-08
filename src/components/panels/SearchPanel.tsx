import { CaseSensitive, Regex, X } from "lucide-react";
import { useCallback, useEffect, useRef, useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import { useSearch } from "@/hooks/useSearch";
import { cn } from "@/lib/utils";
import type { SearchMatch } from "@/types/search";

interface GroupedMatches {
	path: string;
	matches: SearchMatch[];
}

function groupByFile(matches: SearchMatch[]): GroupedMatches[] {
	const map = new Map<string, SearchMatch[]>();
	for (const m of matches) {
		const list = map.get(m.path) ?? [];
		list.push(m);
		map.set(m.path, list);
	}
	return Array.from(map.entries()).map(([path, matches]) => ({
		path,
		matches,
	}));
}

function HighlightedLine({
	line,
	matchStart,
	matchEnd,
}: {
	line: string;
	matchStart: number;
	matchEnd: number;
}) {
	const before = line.slice(0, matchStart);
	const match = line.slice(matchStart, matchEnd);
	const after = line.slice(matchEnd);
	return (
		<span className="truncate">
			{before}
			<span className="bg-yellow-500/40 text-foreground font-semibold">
				{match}
			</span>
			{after}
		</span>
	);
}

export interface SearchPanelProps {
	rootPath: string | null;
	onSelectFileAtLine?: (relativePath: string, line: number) => void;
	focusKey?: number;
}

export function SearchPanel({
	rootPath,
	onSelectFileAtLine,
	focusKey,
}: SearchPanelProps) {
	const { result, loading, error, search, clear } = useSearch(rootPath);
	const [query, setQuery] = useState("");
	const [caseSensitive, setCaseSensitive] = useState(false);
	const [isRegex, setIsRegex] = useState(false);
	const inputRef = useRef<HTMLInputElement>(null);
	const debounceRef = useRef<ReturnType<typeof setTimeout> | null>(null);
	const queryRef = useRef(query);
	queryRef.current = query;

	const triggerSearch = useCallback(
		(value: string) => {
			if (debounceRef.current) clearTimeout(debounceRef.current);
			if (!value.trim()) {
				clear();
				return;
			}
			debounceRef.current = setTimeout(() => {
				search(value, { caseSensitive, isRegex });
			}, 300);
		},
		[search, clear, caseSensitive, isRegex],
	);

	const handleInputChange = useCallback(
		(e: React.ChangeEvent<HTMLInputElement>) => {
			const value = e.target.value;
			setQuery(value);
			triggerSearch(value);
		},
		[triggerSearch],
	);

	const handleClear = useCallback(() => {
		if (debounceRef.current) clearTimeout(debounceRef.current);
		setQuery("");
		clear();
		inputRef.current?.focus();
	}, [clear]);

	useEffect(() => {
		if (queryRef.current.trim()) {
			triggerSearch(queryRef.current);
		}
	}, [triggerSearch]);

	useEffect(() => {
		return () => {
			if (debounceRef.current) clearTimeout(debounceRef.current);
		};
	}, []);

	useEffect(() => {
		if (focusKey != null && focusKey > 0) {
			inputRef.current?.focus();
		}
	}, [focusKey]);

	const grouped = result ? groupByFile(result.matches) : [];
	const fileCount = grouped.length;
	const matchCount = result?.total_matches ?? 0;

	if (!rootPath) {
		return (
			<div className="h-full flex items-center justify-center bg-sidebar">
				<span className="text-sm text-muted-foreground">No folder opened</span>
			</div>
		);
	}

	return (
		<div className="h-full flex flex-col bg-sidebar">
			<div className="flex items-center gap-2 h-[30px] px-3 border-b border-border shrink-0">
				<span className="text-xs font-semibold uppercase tracking-wide truncate">
					Search
				</span>
			</div>

			<div className="px-3 py-2 shrink-0 flex flex-col gap-1.5">
				<div className="flex items-center gap-1">
					<div className="relative flex-1">
						<input
							ref={inputRef}
							type="text"
							className="w-full bg-transparent border border-border rounded px-2 py-1 text-xs outline-none focus:border-primary pr-6"
							placeholder="Search files..."
							value={query}
							onChange={handleInputChange}
							data-testid="search-input"
						/>
						{query && (
							<button
								type="button"
								className="absolute right-1 top-1/2 -translate-y-1/2 p-0.5 text-muted-foreground hover:text-foreground"
								onClick={handleClear}
								title="Clear"
							>
								<X className="h-3 w-3" />
							</button>
						)}
					</div>
					<button
						type="button"
						className={cn(
							"p-1 rounded text-muted-foreground hover:text-foreground transition-colors",
							caseSensitive && "bg-muted text-foreground",
						)}
						onClick={() => setCaseSensitive((v) => !v)}
						title="Match Case"
						data-testid="toggle-case"
					>
						<CaseSensitive className="h-3.5 w-3.5" />
					</button>
					<button
						type="button"
						className={cn(
							"p-1 rounded text-muted-foreground hover:text-foreground transition-colors",
							isRegex && "bg-muted text-foreground",
						)}
						onClick={() => setIsRegex((v) => !v)}
						title="Use Regular Expression"
						data-testid="toggle-regex"
					>
						<Regex className="h-3.5 w-3.5" />
					</button>
				</div>

				{result && (
					<div className="text-[10px] text-muted-foreground">
						{matchCount} results in {fileCount} files
						{result.truncated && " (truncated)"}
					</div>
				)}
				{loading && (
					<div className="text-[10px] text-muted-foreground">Searching...</div>
				)}
				{error && <div className="text-[10px] text-destructive">{error}</div>}
			</div>

			<ScrollArea className="flex-1 min-h-0 [&>[data-slot=scroll-area-viewport]>div]:block!">
				{grouped.map((group) => (
					<div key={group.path}>
						<div className="px-3 py-1 text-[11px] font-semibold text-muted-foreground truncate bg-sidebar-accent/50">
							{group.path}
						</div>
						{group.matches.map((m, i) => (
							<button
								type="button"
								key={`${m.line_number}-${i}`}
								className="flex w-full items-center gap-2 px-4 py-0.5 text-xs hover:bg-sidebar-accent transition-colors text-left"
								onClick={() => onSelectFileAtLine?.(m.path, m.line_number)}
							>
								<span className="text-muted-foreground font-mono w-8 text-right shrink-0">
									{m.line_number}
								</span>
								<HighlightedLine
									line={m.line_content}
									matchStart={m.match_start}
									matchEnd={m.match_end}
								/>
							</button>
						))}
					</div>
				))}
			</ScrollArea>
		</div>
	);
}
