import { AutocompletePopup } from "./AutocompletePopup";

interface MentionPopupProps {
	open: boolean;
	files: string[];
	selectedIndex: number;
	onSelect: (filePath: string) => void;
	onClose: () => void;
	anchorRef: React.RefObject<HTMLElement | null>;
}

export function MentionPopup({
	open,
	files,
	selectedIndex,
	onSelect,
	onClose,
	anchorRef,
}: MentionPopupProps) {
	return (
		<AutocompletePopup
			open={open}
			items={files}
			selectedIndex={selectedIndex}
			onSelect={onSelect}
			onClose={onClose}
			anchorRef={anchorRef}
			getKey={(f) => f}
			renderItem={(f) => (
				<span className="truncate font-mono text-xs">{f}</span>
			)}
			testId="mention-file-list"
		/>
	);
}
