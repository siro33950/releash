import type { SlashCommand } from "@/types/session";
import { AutocompletePopup } from "./AutocompletePopup";

interface SlashCommandPopupProps {
	open: boolean;
	commands: SlashCommand[];
	selectedIndex: number;
	onSelect: (command: SlashCommand) => void;
	onClose: () => void;
	anchorRef: React.RefObject<HTMLElement | null>;
}

export function SlashCommandPopup({
	open,
	commands,
	selectedIndex,
	onSelect,
	onClose,
	anchorRef,
}: SlashCommandPopupProps) {
	return (
		<AutocompletePopup
			open={open}
			items={commands}
			selectedIndex={selectedIndex}
			onSelect={onSelect}
			onClose={onClose}
			anchorRef={anchorRef}
			getKey={(cmd) => cmd.name}
			renderItem={(cmd) => (
				<>
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
				</>
			)}
			testId="slash-command-list"
			itemClassName="flex flex-col"
		/>
	);
}
