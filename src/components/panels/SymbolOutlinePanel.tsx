import { invoke } from "@tauri-apps/api/core";
import {
	Box,
	Braces,
	Code,
	Diamond,
	Hash,
	type LucideIcon,
	Variable,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";

interface DocumentSymbol {
	name: string;
	kind: string;
	line: number;
	column: number;
	end_line: number;
}

interface SymbolOutlinePanelProps {
	filePath: string | null;
	language: string | null;
	onSelectSymbol?: (line: number) => void;
}

const kindIcons: Record<string, LucideIcon> = {
	function: Code,
	method: Code,
	class: Box,
	module: Box,
	interface: Diamond,
	type: Diamond,
	constant: Hash,
	variable: Variable,
	enum: Braces,
	struct: Braces,
	trait: Diamond,
};

function getKindIcon(kind: string): LucideIcon {
	const normalized = kind.toLowerCase();
	return kindIcons[normalized] ?? Code;
}

function getKindColor(kind: string): string {
	const normalized = kind.toLowerCase();
	switch (normalized) {
		case "function":
		case "method":
			return "text-purple-400";
		case "class":
		case "module":
		case "struct":
			return "text-amber-400";
		case "interface":
		case "type":
		case "trait":
			return "text-sky-400";
		case "constant":
		case "enum":
			return "text-emerald-400";
		default:
			return "text-muted-foreground";
	}
}

export function SymbolOutlinePanel({
	filePath,
	language,
	onSelectSymbol,
}: SymbolOutlinePanelProps) {
	const [symbols, setSymbols] = useState<DocumentSymbol[]>([]);
	const [loading, setLoading] = useState(false);

	useEffect(() => {
		if (!filePath || !language) {
			setSymbols([]);
			setLoading(false);
			return;
		}

		let cancelled = false;
		setLoading(true);

		invoke<DocumentSymbol[]>("list_document_symbols", {
			filePath,
			language,
		})
			.then((result) => {
				if (!cancelled) {
					setSymbols(result);
				}
			})
			.catch((err) => {
				if (!cancelled) {
					console.error("[SymbolOutlinePanel] Failed to fetch symbols:", err);
					setSymbols([]);
				}
			})
			.finally(() => {
				if (!cancelled) {
					setLoading(false);
				}
			});

		return () => {
			cancelled = true;
		};
	}, [filePath, language]);

	const handleClick = useCallback(
		(line: number) => {
			onSelectSymbol?.(line);
		},
		[onSelectSymbol],
	);

	if (!filePath) {
		return (
			<div className="flex items-center justify-center h-full text-xs text-muted-foreground">
				ファイルを開いてください
			</div>
		);
	}

	if (loading) {
		return (
			<div className="flex items-center justify-center h-full text-xs text-muted-foreground">
				Loading...
			</div>
		);
	}

	if (symbols.length === 0) {
		return (
			<div className="flex items-center justify-center h-full text-xs text-muted-foreground">
				シンボルが見つかりません
			</div>
		);
	}

	return (
		<ScrollArea className="h-full">
			<div className="py-1">
				{symbols.map((symbol) => {
					const Icon = getKindIcon(symbol.kind);
					const colorClass = getKindColor(symbol.kind);
					return (
						<button
							key={`${symbol.name}-${symbol.line}`}
							type="button"
							className="flex items-center gap-1.5 w-full px-3 py-0.5 text-xs hover:bg-accent/50 cursor-pointer text-left"
							onClick={() => handleClick(symbol.line)}
						>
							<Icon className={`size-3.5 shrink-0 ${colorClass}`} />
							<span className="truncate">{symbol.name}</span>
							<span className="ml-auto text-muted-foreground/60 tabular-nums shrink-0">
								:{symbol.line}
							</span>
						</button>
					);
				})}
			</div>
		</ScrollArea>
	);
}
