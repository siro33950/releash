import {
	AlertDialog,
	AlertDialogAction,
	AlertDialogCancel,
	AlertDialogContent,
	AlertDialogDescription,
	AlertDialogFooter,
	AlertDialogHeader,
	AlertDialogTitle,
} from "@/components/ui/alert-dialog";

interface UnsavedChangesDialogProps {
	open: boolean;
	fileName: string;
	onSave: () => void;
	onDiscard: () => void;
	onCancel: () => void;
}

export function UnsavedChangesDialog({
	open,
	fileName,
	onSave,
	onDiscard,
	onCancel,
}: UnsavedChangesDialogProps) {
	return (
		<AlertDialog open={open} onOpenChange={(o) => !o && onCancel()}>
			<AlertDialogContent>
				<AlertDialogHeader>
					<AlertDialogTitle>Unsaved Changes</AlertDialogTitle>
					<AlertDialogDescription>
						"{fileName}" has unsaved changes. Save them?
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={onCancel}>Cancel</AlertDialogCancel>
					<AlertDialogAction
						onClick={onDiscard}
						className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
					>
						Don't Save
					</AlertDialogAction>
					<AlertDialogAction onClick={onSave}>Save</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
