import { Plus } from "lucide-react";

interface NewWorktreeCardProps {
	onClick: () => void;
}

export function NewWorktreeCard({ onClick }: NewWorktreeCardProps) {
	return (
		<button
			type="button"
			onClick={onClick}
			className="flex flex-col items-center justify-center gap-2 rounded-lg border-2 border-dashed border-border p-4 hover:border-primary/50 hover:bg-accent/50 transition-colors min-h-[140px]"
		>
			<Plus className="size-8 text-muted-foreground" />
			<span className="text-sm text-muted-foreground">New Workspace</span>
		</button>
	);
}
