import { AutocompletePopup } from "./AutocompletePopup";

interface MentionPopupProps {
	open: boolean;
	files: string[];
	selectedIndex: number;
	onSelect: (filePath: string) => void;
	onClose: () => void;
	children: React.ReactNode;
}

export function MentionPopup({
	open,
	files,
	selectedIndex,
	onSelect,
	onClose,
	children,
}: MentionPopupProps) {
	return (
		<AutocompletePopup
			open={open}
			items={files}
			selectedIndex={selectedIndex}
			onSelect={onSelect}
			onClose={onClose}
			getKey={(f) => f}
			renderItem={(f) => (
				<span className="truncate font-mono text-xs">{f}</span>
			)}
			testId="mention-file-list"
		>
			{children}
		</AutocompletePopup>
	);
}
