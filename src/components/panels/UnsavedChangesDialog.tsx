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
					<AlertDialogTitle>未保存の変更</AlertDialogTitle>
					<AlertDialogDescription>
						「{fileName}」に未保存の変更があります。保存しますか？
					</AlertDialogDescription>
				</AlertDialogHeader>
				<AlertDialogFooter>
					<AlertDialogCancel onClick={onCancel}>キャンセル</AlertDialogCancel>
					<AlertDialogAction
						onClick={onDiscard}
						className="bg-destructive text-destructive-foreground hover:bg-destructive/90"
					>
						保存しない
					</AlertDialogAction>
					<AlertDialogAction onClick={onSave}>保存</AlertDialogAction>
				</AlertDialogFooter>
			</AlertDialogContent>
		</AlertDialog>
	);
}
