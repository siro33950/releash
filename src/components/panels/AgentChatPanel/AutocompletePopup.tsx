import { useCallback } from "react";
import {
	Popover,
	PopoverAnchor,
	PopoverContent,
} from "@/components/ui/popover";

interface AutocompletePopupProps<T> {
	open: boolean;
	items: T[];
	selectedIndex: number;
	onSelect: (item: T) => void;
	onClose: () => void;
	children: React.ReactNode;
	getKey: (item: T) => string;
	renderItem: (item: T) => React.ReactNode;
	testId: string;
	itemClassName?: string;
}

export function AutocompletePopup<T>({
	open,
	items,
	selectedIndex,
	onSelect,
	onClose,
	children,
	getKey,
	renderItem,
	testId,
	itemClassName = "flex items-center gap-2",
}: AutocompletePopupProps<T>) {
	const selectedRef = useCallback((node: HTMLDivElement | null) => {
		if (node && typeof node.scrollIntoView === "function") {
			node.scrollIntoView({ block: "nearest" });
		}
	}, []);

	return (
		<Popover
			open={open && items.length > 0}
			onOpenChange={(o) => !o && onClose()}
		>
			<PopoverAnchor asChild>{children}</PopoverAnchor>
			<PopoverContent
				side="top"
				align="start"
				className="w-[var(--radix-popover-trigger-width)] max-h-[240px] overflow-y-auto p-1"
				onOpenAutoFocus={(e) => e.preventDefault()}
			>
				<div role="listbox" data-testid={testId}>
					{items.map((item, i) => (
						<div
							key={getKey(item)}
							ref={i === selectedIndex ? selectedRef : undefined}
							role="option"
							tabIndex={-1}
							aria-selected={i === selectedIndex}
							data-selected={i === selectedIndex}
							className={`${itemClassName} px-2 py-1.5 rounded-sm text-sm cursor-pointer data-[selected=true]:bg-foreground/10 data-[selected=true]:text-foreground`}
							onMouseDown={(e) => {
								e.preventDefault();
								onSelect(item);
							}}
						>
							{renderItem(item)}
						</div>
					))}
				</div>
			</PopoverContent>
		</Popover>
	);
}
