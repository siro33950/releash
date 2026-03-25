import { useCallback } from "react";
import {
	Popover,
	PopoverAnchor,
	PopoverContent,
} from "@/components/ui/popover";
import type { SlashCommand } from "@/hooks/useSlashCommands";

interface SlashCommandPopupProps {
	open: boolean;
	commands: SlashCommand[];
	selectedIndex: number;
	onSelect: (command: SlashCommand) => void;
	onClose: () => void;
	children: React.ReactNode;
}

export function SlashCommandPopup({
	open,
	commands,
	selectedIndex,
	onSelect,
	onClose,
	children,
}: SlashCommandPopupProps) {
	const selectedRef = useCallback((node: HTMLDivElement | null) => {
		if (node && typeof node.scrollIntoView === "function") {
			node.scrollIntoView({ block: "nearest" });
		}
	}, []);

	return (
		<Popover
			open={open && commands.length > 0}
			onOpenChange={(o) => !o && onClose()}
		>
			<PopoverAnchor asChild>{children}</PopoverAnchor>
			<PopoverContent
				side="top"
				align="start"
				className="w-[var(--radix-popover-trigger-width)] max-h-[240px] overflow-y-auto p-1"
				onOpenAutoFocus={(e) => e.preventDefault()}
			>
				<div role="listbox" data-testid="slash-command-list">
					{commands.map((cmd, i) => (
						<div
							key={cmd.name}
							ref={i === selectedIndex ? selectedRef : undefined}
							role="option"
							tabIndex={-1}
							aria-selected={i === selectedIndex}
							data-selected={i === selectedIndex}
							className="flex flex-col px-2 py-1.5 rounded-sm text-sm cursor-pointer data-[selected=true]:bg-foreground/10 data-[selected=true]:text-foreground"
							onMouseDown={(e) => {
								e.preventDefault();
								onSelect(cmd);
							}}
						>
							<span className="font-medium">
								/{cmd.name}
								{cmd.argumentHint ? (
									<span className="ml-1 text-muted-foreground font-normal">
										{cmd.argumentHint}
									</span>
								) : null}
							</span>
							<span className="text-xs text-muted-foreground">
								{cmd.description}
							</span>
						</div>
					))}
				</div>
			</PopoverContent>
		</Popover>
	);
}
